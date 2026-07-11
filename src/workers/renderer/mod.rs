//! TODO: translucency
//!
//! pass architecture:
//! - compute culling pass: determine which chunks will be rendered
//! - depth prepass: conservative rasterized depth for all visible chunks (no portals)
//! - primary pass: raymarch to build visbuf with voxel IDs (go through portals, report unloaded chunks)
//! - collect pass: build gpu hash table of voxel IDs
//! - shade pass: dispatch shading per visible voxel (not per pixel)
//! - resolve pass: apply colors to visbuf based on shading results
//! - blit pass: blit shaded image to swapchain
//! - egui pass: draw ui
//!
//! 64-bit voxel ID structure:
//! - entity flag: 1 bit
//! - if entity flag is set:
//!   - entity ID
//!   - entity-local coordinates
//! - else:
//!   - active chunk index (index into array of loaded chunks, not GUID)
//!   - chunk-local coordinates (24 bits)
mod frames;
mod passes;
mod resources;

use anyhow::Result;
use bytemuck::bytes_of;
use winit::dpi::PhysicalSize;
use std::{sync::Arc, time::Duration};

use crate::{
    gpu::{ctx::{Destroy, VulkanCtx}, task::Task},
    sync::{chan::SafeSend, worker::{MessageHandler, TickTiming, Worker}},
    workers::msgs::{
        CameraState,
        EguiUpdate,
        RendererMsg,
        RendererRxBundle,
        RendererTelemetry,
        RendererToApp,
    }, world::{ChunkAABBPtr, ChunkMetadata, ChunkState, Node},
};
use frames::{FrameManager, RenderTarget};
use passes::Passes;
use resources::{Resources, ResourceView};

pub struct RendererInit {
    pub sz: PhysicalSize<u32>,
    pub ctx: Arc<VulkanCtx>,
    pub app_tx: RendererToApp,
    pub camera_rx: triple_buffer::Output<CameraState>,
}

pub struct RendererState {
    manager: FrameManager,
    resources: Resources,
    passes: Passes,

    app_tx: RendererToApp,
    ctx: Arc<VulkanCtx>,

    // TODO(loader): remove this when chunk streaming is in
    staged_chunk: bool,
}

impl Drop for RendererState {
    fn drop(&mut self) {
        // ctx is dropped by the compiler after this is done
        unsafe {
            let _ = self.ctx.dev.device_wait_idle();

            self.passes.destroy(&self.ctx);
            self.resources.destroy(&self.ctx);
            self.manager.destroy(&self.ctx);
        }
    }
}

#[derive(Default)]
pub struct Renderer;

pub struct FrameCtx<'a> {
    pub resources: ResourceView<'a>,
    pub in_flight: RenderTarget<'a>,
    pub frame_idx: usize,
    pub time: f64, // time since engine start
    pub dt: f32, // time since last frame
}

impl Worker for Renderer {
    type Init = RendererInit;
    type State = RendererState;
    type Channels = RendererRxBundle;

    fn on_resume(&self, init: Self::Init) -> Result<Self::State> {
        let ctx = init.ctx;

        let manager = FrameManager::new(ctx.clone(), init.sz)?;
        let resources = Resources::new(ctx.clone(), init.sz, init.camera_rx)?;
        let passes = Passes::new(&ctx, &resources, manager.swapchain.fmt)?;

        Ok(RendererState {
            manager,
            resources,
            passes,

            app_tx: init.app_tx,
            ctx,

            staged_chunk: false,
        })
    }

    fn on_tick(
        this: &mut Self::State,
        tick: &TickTiming,
    ) -> Result<()> {
        let dt = tick.dt.as_secs_f32();

        this.app_tx.telemetry.send(RendererTelemetry { frame_secs: dt });

        let Some(mut target) = this.manager.begin_frame(&this.ctx)? else {
            if this.manager.is_minimized() {
                // sleep to avoid busy wait
                std::thread::sleep(Duration::from_millis(50));
            }
            return Ok(());
        };

        if target.did_resize {
            unsafe {
                this.ctx.dev.device_wait_idle()?;
                this.resources.resize(&this.ctx, target.img.extent)?;
            }
        }

        // TODO(loader): remove this when chunk streaming is in
        if !this.staged_chunk {
            this.staged_chunk = true;

            let node = Node::new().with_child_mask(0).with_mat_id(1);
            let chunk_meta = ChunkMetadata::new().with_state(ChunkState::Loaded);

            unsafe {
                this.ctx.dev.cmd_copy_buffer(
                    target.cmd,
                    target.arena.buf,
                    this.resources.voxel_data.buf,
                    &[target.arena.stage(&node).get_buf_copy(0)],
                );

                let roots_src = target.arena.stage(&this.resources.voxel_data.bda);
                let meta_src = target.arena.stage(&chunk_meta);
                let aabb_src = target.arena.stage(&ChunkAABBPtr::new());

                this.ctx.dev.cmd_copy_buffer(
                    target.cmd,
                    target.arena.buf,
                    this.resources.chunk_headers.buf(),
                    &[
                        this.resources.chunk_headers.roots.copy_from(&roots_src),
                        this.resources.chunk_headers.metadata.copy_from(&meta_src),
                        this.resources.chunk_headers.aabb_ptrs.copy_from(&aabb_src),
                    ],
                );
            }
        }

        unsafe {
            this.passes.execute(&this.ctx, &mut FrameCtx {
                resources: this.resources.view(&mut target),
                in_flight: target,
                frame_idx: tick.n,
                time: tick.since_wake.as_secs_f64(),
                dt,
            })?;
        }

        this.manager.submit_frame(&this.ctx)
    }
}

impl MessageHandler<RendererMsg> for RendererState {
    fn handle(&mut self, msg: RendererMsg) {
        match msg {
            RendererMsg::Resized(sz) => self.manager.request_resize(sz),
        }
    }
}

impl MessageHandler<EguiUpdate> for RendererState {
    fn handle(&mut self, msg: EguiUpdate) {
        self.passes.handle(msg);
    }
}
