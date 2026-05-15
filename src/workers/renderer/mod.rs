//! TODO: translucency
//!
//! pass architecture:
//! - compute culling pass: determine which chunks will be rendered
//! - depth prepass: conservative rasterized depth for all visible chunks (no portals)
//! - beam prepass: low-resolution beam tracing for refining depth map (portals)
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

use ash::vk;
use winit::dpi::PhysicalSize;
use std::{sync::Arc, time::Duration};

use crate::{
    gpu::{ctx::{Destroy, VulkanCtx}, sync::ImageBarrierExt, task::Task},
    sync::{chan::SafeSend, worker::{MessageHandler, Worker}},
};
use super::msgs::{
    CameraState,
    EguiUpdate,
    RendererMsg,
    RendererRxBundle,
    RendererTelemetry,
    RendererToApp,
};
use frames::{FrameManager, RenderTarget};
use passes::{
    blit::BlitPass,
    depth::DepthPrepass,
    egui::EguiPass,
    frame::ClearPass,
    primary::PrimaryPass,
};
use resources::{Resources, ResourceView, COLOR_SUBRES_RANGE, DEPTH_SUBRES_RANGE};

pub struct RendererInit {
    pub sz: PhysicalSize<u32>,
    pub ctx: Arc<VulkanCtx>,
    pub app_tx: RendererToApp,
    pub camera_rx: triple_buffer::Output<CameraState>,
}

pub struct RendererState {
    manager: FrameManager,
    resources: Resources,

    clear_pass: ClearPass,
    depth_prepass: DepthPrepass,
    primary_pass: PrimaryPass,
    blit_pass: BlitPass,
    egui_pass: EguiPass,

    app_tx: RendererToApp,
    ctx: Arc<VulkanCtx>,
}

impl Drop for RendererState {
    fn drop(&mut self) {
        // ctx is dropped by the compiler after this is done
        unsafe {
            let _ = self.ctx.dev.device_wait_idle();

            self.egui_pass.destroy(&self.ctx);
            self.blit_pass.destroy(&self.ctx);
            self.primary_pass.destroy(&self.ctx);
            self.depth_prepass.destroy(&self.ctx);
            self.clear_pass.destroy(&self.ctx);

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

    fn on_resume(&self, init: Self::Init) -> anyhow::Result<Self::State> {
        let ctx = init.ctx;

        let manager = FrameManager::new(ctx.clone(), init.sz)?;
        let resources = Resources::new(ctx.clone(), init.sz, init.camera_rx)?;

        let clear_pass = ClearPass;
        let depth_prepass = DepthPrepass::new(&ctx)?;
        let primary_pass = PrimaryPass::new(&ctx, resources.desc.get_bind_info())?;
        let blit_pass = BlitPass;
        let egui_pass = EguiPass::new(&ctx, manager.swapchain.fmt)?;

        Ok(RendererState {
            manager,
            resources,

            clear_pass,
            depth_prepass,
            primary_pass,
            blit_pass,
            egui_pass,

            app_tx: init.app_tx,
            ctx,
        })
    }

    fn on_tick(
        this: &mut Self::State,
        since_wake: Duration,
        since_tick: Duration,
        tick: usize,
    ) -> anyhow::Result<()> {
        let dt = since_tick.as_secs_f32();

        this.app_tx.telemetry.send(RendererTelemetry { frame_time: dt });

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

        let mut frame = FrameCtx {
            resources: this.resources.view(&mut target),
            in_flight: target,
            frame_idx: tick,
            time: since_wake.as_secs_f64(),
            dt,
        };

        unsafe {
            let cmd = frame.in_flight.cmd;

            let mut swap_barrier = frame.in_flight.base_barrier
                .begin(
                    vk::ImageLayout::COLOR_ATTACHMENT_OPTIMAL,
                    vk::PipelineStageFlags2::COLOR_ATTACHMENT_OUTPUT,
                    vk::AccessFlags2::COLOR_ATTACHMENT_WRITE,
                )
                // don't use TOP_OF_PIPE default for swapchain images
                .src_stage_mask(vk::PipelineStageFlags2::COLOR_ATTACHMENT_OUTPUT);
            this.ctx.barrier(cmd, [swap_barrier]);
            this.clear_pass.execute(&this.ctx, &mut frame)?;

            let mut depth_barrier = vk::ImageMemoryBarrier2::default()
                .image(frame.resources.depth.img)
                .subresource_range(DEPTH_SUBRES_RANGE)
                .begin(
                    vk::ImageLayout::DEPTH_ATTACHMENT_OPTIMAL,
                    vk::PipelineStageFlags2::EARLY_FRAGMENT_TESTS
                    | vk::PipelineStageFlags2::LATE_FRAGMENT_TESTS,
                    vk::AccessFlags2::DEPTH_STENCIL_ATTACHMENT_WRITE,
                );
            this.ctx.barrier(cmd, [depth_barrier]);
            this.depth_prepass.execute(&this.ctx, &mut frame)?;

            let mut color_barrier = vk::ImageMemoryBarrier2::default()
                .image(frame.resources.color.img)
                .subresource_range(COLOR_SUBRES_RANGE)
                .begin(
                    vk::ImageLayout::GENERAL,
                    vk::PipelineStageFlags2::COMPUTE_SHADER,
                    vk::AccessFlags2::SHADER_WRITE,
                );
            depth_barrier.transition(
                vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL,
                vk::PipelineStageFlags2::COMPUTE_SHADER,
                vk::AccessFlags2::SHADER_SAMPLED_READ,
            );
            this.ctx.barrier(cmd, [depth_barrier, color_barrier]);
            this.primary_pass.execute(&this.ctx, &mut frame)?;

            color_barrier.transition(
                vk::ImageLayout::TRANSFER_SRC_OPTIMAL,
                vk::PipelineStageFlags2::BLIT,
                vk::AccessFlags2::TRANSFER_READ,
            );
            swap_barrier.transition(
                vk::ImageLayout::TRANSFER_DST_OPTIMAL,
                vk::PipelineStageFlags2::BLIT,
                vk::AccessFlags2::TRANSFER_WRITE,
            );
            this.ctx.barrier(cmd, [color_barrier, swap_barrier]);
            this.blit_pass.execute(&this.ctx, &mut frame)?;

            swap_barrier.transition(
                vk::ImageLayout::COLOR_ATTACHMENT_OPTIMAL,
                vk::PipelineStageFlags2::COLOR_ATTACHMENT_OUTPUT,
                vk::AccessFlags2::COLOR_ATTACHMENT_READ
                | vk::AccessFlags2::COLOR_ATTACHMENT_WRITE,
            );
            this.ctx.barrier(cmd, [swap_barrier]);
            this.egui_pass.execute(&this.ctx, &mut frame)?;

            swap_barrier.transition(
                vk::ImageLayout::PRESENT_SRC_KHR,
                vk::PipelineStageFlags2::BOTTOM_OF_PIPE,
                vk::AccessFlags2::NONE,
            );
            this.ctx.barrier(cmd, [swap_barrier]);
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
        self.egui_pass.apply_update(msg);
    }
}
