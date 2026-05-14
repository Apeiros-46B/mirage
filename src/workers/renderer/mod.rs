mod frames;
mod passes;

use ash::vk;
use triple_buffer::Output;
use winit::dpi::PhysicalSize;
use std::{sync::Arc, time::Duration};

use crate::{
    gpu::{
        ctx::{Destroy, VulkanCtx},
        desc::DescriptorAllocator,
        img::AllocatedImage,
        sync::ImageBarrierExt,
        task::Task,
    },
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
use passes::{
    blit::BlitPass,
    depth::DepthPrepass,
    egui::EguiPass,
    frame::ClearPass,
    primary::PrimaryPass,
};
use frames::{FrameManager, PerFrame, RenderTarget};

pub struct RendererInit {
    pub sz: PhysicalSize<u32>,
    pub ctx: Arc<VulkanCtx>,
    pub app_tx: RendererToApp,
    pub camera_rx: Output<CameraState>,
}

pub struct RendererState {
    desc_alloc: DescriptorAllocator,
    manager: FrameManager,

    depth_targets: PerFrame<AllocatedImage>,
    color_targets: PerFrame<AllocatedImage>,

    depth_idxs: PerFrame<u32>,
    color_storage_idxs: PerFrame<u32>,

    clear_pass: ClearPass,
    depth_prepass: DepthPrepass,
    primary_pass: PrimaryPass,
    blit_pass: BlitPass,
    egui_pass: EguiPass,

    app_tx: RendererToApp,
    camera_rx: Output<CameraState>,
    ctx: Arc<VulkanCtx>,
}

impl Drop for RendererState {
    fn drop(&mut self) {
        // ctx is dropped by the compiler after this is done
        unsafe {
            let _ = self.ctx.dev.device_wait_idle();

            self.egui_pass.destroy(&self.ctx);
            self.primary_pass.destroy(&self.ctx);
            self.depth_prepass.destroy(&self.ctx);

            self.depth_targets.destroy(&self.ctx);
            self.color_targets.destroy(&self.ctx);

            self.manager.destroy(&self.ctx);
        }
    }
}

#[derive(Default)]
pub struct Renderer;

pub struct FrameCtx<'a> {
    pub in_flight: RenderTarget<'a>,
    pub frame_idx: usize,
    pub time: f64, // time since engine start
    pub dt: f32, // time since last frame

    pub camera_bda: u64, // pointer to CameraState

    pub depth: &'a AllocatedImage,
    pub color: &'a AllocatedImage,
    pub depth_idx: u32,
    pub color_storage_idx: u32,
}

impl Worker for Renderer {
    type Init = RendererInit;
    type State = RendererState;
    type Channels = RendererRxBundle;

    fn on_resume(&self, init: Self::Init) -> anyhow::Result<Self::State> {
        let ctx = init.ctx;
        let extent = vk::Extent2D { width: init.sz.width, height: init.sz.height };
        let mut desc_alloc = DescriptorAllocator::new(ctx.clone())?;
        let manager = FrameManager::new(ctx.clone(), init.sz)?;

        let depth_targets = make_depth_targets(&ctx, extent)?;
        let color_targets = make_color_targets(&ctx, extent)?;

        let depth_idxs = desc_alloc.alloc_sampled_imgs(&depth_targets).into();
        let color_storage_idxs = desc_alloc.alloc_storage_imgs(&color_targets).into();

        let clear_pass = ClearPass;
        let depth_prepass = DepthPrepass::new(&ctx)?;
        let primary_pass = PrimaryPass::new(&ctx, desc_alloc.get_bind_info())?;
        let blit_pass = BlitPass;
        let egui_pass = EguiPass::new(&ctx, manager.swapchain.fmt)?;

        Ok(RendererState {
            desc_alloc,
            manager,

            depth_targets,
            color_targets,

            depth_idxs,
            color_storage_idxs,

            clear_pass,
            depth_prepass,
            primary_pass,
            blit_pass,
            egui_pass,

            app_tx: init.app_tx,
            camera_rx: init.camera_rx,
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

        let Some(target) = this.manager.begin_frame(&this.ctx)? else {
            if this.manager.is_minimized() {
                // sleep to avoid busy wait
                std::thread::sleep(Duration::from_millis(50));
            }
            return Ok(());
        };

        if target.did_resize {
            unsafe {
                this.ctx.dev.device_wait_idle()?;

                this.depth_targets.destroy(&this.ctx);
                this.depth_targets = make_depth_targets(&this.ctx, target.img.extent)?;
                this.desc_alloc.update_sampled_imgs(
                    this.depth_idxs[0],
                    &this.depth_targets,
                );

                this.color_targets.destroy(&this.ctx);
                this.color_targets = make_color_targets(&this.ctx, target.img.extent)?;
                this.desc_alloc.update_storage_imgs(
                    this.color_storage_idxs[0],
                    &this.color_targets,
                );
            }
        }

        let mut camera = target.arena.alloc_of::<CameraState>();
        unsafe { camera.write(this.camera_rx.read()); }

        let fif = target.idx;
        let mut frame = FrameCtx {
            in_flight: target,
            frame_idx: tick,
            time: since_wake.as_secs_f64(),
            dt,

            camera_bda: camera.bda,

            depth: &this.depth_targets[fif],
            color: &this.color_targets[fif],

            depth_idx: this.depth_idxs[fif],
            color_storage_idx: this.color_storage_idxs[fif],
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
                .image(frame.depth.img)
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
                .image(frame.color.img)
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

const DEPTH_SUBRES_RANGE: vk::ImageSubresourceRange = vk::ImageSubresourceRange {
    aspect_mask: vk::ImageAspectFlags::DEPTH,
    base_mip_level: 0,
    level_count: 1,
    base_array_layer: 0,
    layer_count: 1,
};

fn make_depth_targets(
    ctx: &VulkanCtx,
    extent: vk::Extent2D,
) -> anyhow::Result<PerFrame<AllocatedImage>> {
    PerFrame::try_init(|_| {
        AllocatedImage::new(
            ctx,
            extent.into(),
            vk::Format::D32_SFLOAT,
            vk::ImageUsageFlags::DEPTH_STENCIL_ATTACHMENT | vk::ImageUsageFlags::SAMPLED,
            DEPTH_SUBRES_RANGE,
            "depth_prepass_out",
        )
    })
}

const COLOR_SUBRES_RANGE: vk::ImageSubresourceRange = vk::ImageSubresourceRange {
    aspect_mask: vk::ImageAspectFlags::COLOR,
    base_mip_level: 0,
    level_count: 1,
    base_array_layer: 0,
    layer_count: 1,
};

fn make_color_targets(
    ctx: &VulkanCtx,
    extent: vk::Extent2D,
) -> anyhow::Result<PerFrame<AllocatedImage>> {
    PerFrame::try_init(|_| {
        AllocatedImage::new(
            ctx,
            extent.into(),
            vk::Format::R8G8B8A8_UNORM,
            vk::ImageUsageFlags::STORAGE | vk::ImageUsageFlags::TRANSFER_SRC,
            COLOR_SUBRES_RANGE,
            "primary_pass_out",
        )
    })
}
