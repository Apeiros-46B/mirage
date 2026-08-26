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
use ash::vk;
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
    world::BuiltWorld,
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
    pending_world: Option<BuiltWorld>,
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
        let world = BuiltWorld::new();
        let num_chunks = world.chunk_metadata.len() as u32 - 1;
        let resources = Resources::new(
            ctx.clone(),
            init.sz,
            init.camera_rx,
            &world.chunk_directory,
        )?;
        let passes = Passes::new(&ctx, &resources, manager.swapchain.fmt)?;

        Ok(RendererState {
            manager,
            resources,
            passes,

            app_tx: init.app_tx,
            ctx,

            pending_world: Some(world),
            num_chunks,
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
        if let Some(world) = this.pending_world.take() {
            let roots = world.chunk_root_offsets
                .iter()
                .enumerate()
                .map(|(chunk_id, offset)| {
                    if chunk_id == 0 {
                        0
                    } else {
                        this.resources.voxel_data.bda + *offset
                    }
                })
                .collect::<Vec<_>>();

            unsafe {
                let nodes_stage = target.arena.stage_slice(&world.nodes);

                this.ctx.dev.cmd_copy_buffer(
                    target.cmd,
                    target.arena.buf,
                    this.resources.voxel_data.buf,
                    &[this.resources.voxel_data.copy_from(&nodes_stage)],
                );

                let roots_stage = target.arena.stage_slice(&roots);
                let meta_stage = target.arena.stage_slice(&world.chunk_metadata);

                this.ctx.dev.cmd_copy_buffer(
                    target.cmd,
                    target.arena.buf,
                    this.resources.chunk_headers.buf(),
                    &[
                        this.resources.chunk_headers.roots.copy_from(&roots_stage),
                        this.resources.chunk_headers.metadata.copy_from(&meta_stage),
                    ],
                );

                let pages_stage = target.arena.stage_slice(&world.chunk_directory.pages);
                let directory_roots_stage =
                    target.arena.stage_slice(&world.chunk_directory.roots);

                this.ctx.dev.cmd_copy_buffer(
                    target.cmd,
                    target.arena.buf,
                    this.resources.chunk_directory.buf(),
                    &[
                        this.resources.chunk_directory.pages.copy_from(&pages_stage),
                        this.resources.chunk_directory.roots.copy_from(
                            &directory_roots_stage,
                        ),
                    ],
                );

                let upload_barrier = vk::MemoryBarrier2::default()
                    .src_stage_mask(vk::PipelineStageFlags2::COPY)
                    .src_access_mask(vk::AccessFlags2::TRANSFER_WRITE)
                    .dst_stage_mask(
                        vk::PipelineStageFlags2::VERTEX_SHADER
                        | vk::PipelineStageFlags2::COMPUTE_SHADER,
                    )
                    .dst_access_mask(vk::AccessFlags2::SHADER_READ);
                let upload_barriers = [upload_barrier];
                let dependency = vk::DependencyInfo::default()
                    .memory_barriers(&upload_barriers);
                this.ctx.dev.cmd_pipeline_barrier2(target.cmd, &dependency);
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
