//! TODO: translucency
//!
//! pass architecture:
//! - compute culling pass: determine which chunks will be rendered
//! - depth prepass: conservative rasterized depth for all visible chunks (no portals)
//! - primary pass: raymarch to build visbuf with voxel IDs (go through portals, report unloaded chunks)
//! - collect pass: build deduplicated list of voxel IDs
//! - shade pass: dispatch shading per visible voxel (not per pixel)
//! - resolve pass: apply colors to visbuf based on shading results
//! - blit pass: blit shaded image to swapchain
//! - egui pass: draw ui
mod frames;
mod passes;
mod resources;

use anyhow::Result;
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
    },
    world::{build_64_tree, build_occupancy_grid, ChunkMetadata, ChunkState},
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
    num_chunks: u32,
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
            num_chunks: 0,
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

            let grid = build_occupancy_grid(|x, y, z| {
                let x = i32::from(x) - 128;
                let y = i32::from(y) - 128;
                let z = i32::from(z) - 128;
                x * x + y * y + z * z < 128 * 128
            });
            let nodes = build_64_tree(&grid);

            let root = this.resources.voxel_data.bda;
            let meta = ChunkMetadata::new().with_state(ChunkState::Loaded);

            let roots_slice = [root, root];
            let meta_slice = [
                meta.with_x(0).with_y(0).with_z(0),
                meta.with_x(1).with_y(1).with_z(1),
            ];
            this.num_chunks = meta_slice.len() as u32;

            unsafe {
                let nodes_stage = target.arena.stage_slice(&nodes);

                this.ctx.dev.cmd_copy_buffer(
                    target.cmd,
                    target.arena.buf,
                    this.resources.voxel_data.buf,
                    &[this.resources.voxel_data.copy_from(&nodes_stage)],
                );

                let roots_stage = target.arena.stage_slice(&roots_slice);
                let meta_stage = target.arena.stage_slice(&meta_slice);

                this.ctx.dev.cmd_copy_buffer(
                    target.cmd,
                    target.arena.buf,
                    this.resources.chunk_headers.buf(),
                    &[
                        this.resources.chunk_headers.roots.copy_from(&roots_stage),
                        this.resources.chunk_headers.metadata.copy_from(&meta_stage),
                    ],
                );
            }
        }

        unsafe {
            this.passes.execute(&this.ctx, &mut FrameCtx {
                resources: this.resources.view(&mut target, this.num_chunks),
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
