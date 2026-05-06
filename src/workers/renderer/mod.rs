mod frames;
mod passes;

use ash::vk;
use triple_buffer::Output;
use winit::dpi::PhysicalSize;
use std::{sync::Arc, time::Duration};

use crate::{
    gpu::{ctx::{Destroy, VulkanCtx}, img::AllocatedImage, task::Task},
    sync::{chan::SafeSend, worker::{MessageHandler, Worker}},
    workers::renderer::frames::PerFrame,
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
    depth::{DepthPrepass, make_depth_targets},
    egui::EguiPass,
    frame::{ClearPass, PresentPass},
    primary::{PrimaryPass, make_color_targets},
};
use frames::{FrameManager, RenderTarget};

pub struct RendererInit {
    pub sz: PhysicalSize<u32>,
    pub ctx: Arc<VulkanCtx>,
    pub app_tx: RendererToApp,
    pub camera_rx: Output<CameraState>,
}

pub struct RendererState {
    manager: FrameManager,

    depth_targets: PerFrame<AllocatedImage>,
    color_targets: PerFrame<AllocatedImage>,

    clear_pass: ClearPass,
    depth_prepass: DepthPrepass,
    primary_pass: PrimaryPass,
    egui_pass: EguiPass,
    present_pass: PresentPass,

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

    pub depth_img: vk::Image,
    pub depth_view: vk::ImageView,
    pub color_img: vk::Image,
    pub color_view: vk::ImageView,
}

impl Worker for Renderer {
    type Init = RendererInit;
    type State = RendererState;
    type Channels = RendererRxBundle;

    fn on_resume(&self, init: Self::Init) -> anyhow::Result<Self::State> {
        let ctx = init.ctx;
        let extent = vk::Extent2D { width: init.sz.width, height: init.sz.height };
        let manager = FrameManager::new(ctx.clone(), init.sz)?;

        let depth_targets = make_depth_targets(&ctx, extent)?;
        let color_targets = make_depth_targets(&ctx, extent)?;

        let clear_pass = ClearPass;
        let depth_prepass = DepthPrepass::new(&ctx)?;
        let primary_pass = PrimaryPass::new(&ctx)?;
        let egui_pass = EguiPass::new(&ctx, manager.swapchain.fmt)?;
        let present_pass = PresentPass;

        Ok(RendererState {
            manager,

            depth_targets,
            color_targets,

            clear_pass,
            depth_prepass,
            primary_pass,
            egui_pass,
            present_pass,

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
                this.depth_targets.destroy(&this.ctx);
                this.depth_targets = make_depth_targets(&this.ctx, target.img.extent)?;

                this.color_targets.destroy(&this.ctx);
                this.color_targets = make_color_targets(&this.ctx, target.img.extent)?;
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

            // TODO maybe just pass depth: AllocatedImage etc. need to doublecheck borrowing but it should be fine
            depth_img: this.depth_targets[fif].img,
            depth_view: this.depth_targets[fif].view,
            color_img: this.color_targets[fif].img,
            color_view: this.color_targets[fif].view,
        };

        unsafe {
            this.clear_pass.execute(&this.ctx, &mut frame)?;
            this.depth_prepass.execute(&this.ctx, &mut frame)?;
            this.primary_pass.execute(&this.ctx, &mut frame)?;

            this.egui_pass.execute(&this.ctx, &mut frame)?;
            this.present_pass.execute(&this.ctx, &mut frame)?;
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
