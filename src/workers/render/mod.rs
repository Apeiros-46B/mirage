mod frames;
mod passes;

use ash::vk;
use std::{sync::Arc, time::Duration};

use crate::{
    gpu::{ctx::VulkanCtx, task::Task},
    sync::{chan::SafeSend, worker::{MessageHandler, Worker}},
};
use super::msgs::{
    EguiUpdate,
    RendererInit,
    RendererMsg,
    RendererRxBundle,
    RendererTelemetry,
    RendererToApp,
};
use passes::{
    depth::DepthPrepass,
    egui::EguiPass,
    frame::{ClearPass, FrameCtx, PresentPass},
};
use frames::FrameManager;

pub struct RendererState {
    frames: FrameManager,

    clear_pass: ClearPass,
    depth_prepass: DepthPrepass,
    present_pass: PresentPass,
    egui_pass: EguiPass,

    app_tx: RendererToApp,
    ctx: Arc<VulkanCtx>,
}

impl Drop for RendererState {
    fn drop(&mut self) {
        // ctx is dropped by the compiler after this is done
        unsafe {
            let _ = self.ctx.dev.device_wait_idle();
            self.frames.destroy(&self.ctx);
        }
    }
}

#[derive(Default)]
pub struct Renderer;

impl Worker for Renderer {
    type Init = RendererInit;
    type State = RendererState;
    type Channels = RendererRxBundle;

    fn on_resume(&self, init: Self::Init) -> anyhow::Result<Self::State> {
        let ctx = init.ctx;
        let app_tx = init.app_tx;
        let extent = vk::Extent2D { width: init.sz.width, height: init.sz.height };
        let frames = FrameManager::new(ctx.clone(), init.sz)?;

        let clear_pass = ClearPass;
        let depth_prepass = DepthPrepass::new(&ctx, extent)?;
        let present_pass = PresentPass;
        let egui_pass = EguiPass::new(&ctx, frames.swapchain.fmt)?;

        Ok(RendererState {
            frames,
            clear_pass,
            depth_prepass,
            present_pass,
            egui_pass,
            app_tx,
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

        let Some(target) = this.frames.begin_frame(&this.ctx)? else {
            if this.frames.is_minimized() {
                // sleep to avoid busy wait
                std::thread::sleep(Duration::from_millis(50));
            }
            return Ok(());
        };

        if target.did_resize {
            this.clear_pass.resize(&this.ctx, target.img.extent)?;
            this.present_pass.resize(&this.ctx, target.img.extent)?;
            this.depth_prepass.resize(&this.ctx, target.img.extent)?;
            this.egui_pass.resize(&this.ctx, target.img.extent)?;
        }

        let mut frame = FrameCtx {
            in_flight: target,
            frame_idx: tick,
            time: since_wake.as_secs_f64(),
            dt,
        };

        unsafe {
            this.clear_pass.execute(&this.ctx, &mut frame)?;
            this.depth_prepass.execute(&this.ctx, &mut frame)?;
            this.present_pass.execute(&this.ctx, &mut frame)?;
            this.egui_pass.execute(&this.ctx, &mut frame)?;
        }

        this.frames.submit_frame(&this.ctx)
    }
}

impl MessageHandler<RendererMsg> for RendererState {
    fn handle(&mut self, msg: RendererMsg) {
        match msg {
            RendererMsg::Resized(sz) => self.frames.request_resize(sz),
        }
    }
}

impl MessageHandler<EguiUpdate> for RendererState {
    fn handle(&mut self, msg: EguiUpdate) {
        self.egui_pass.apply_update(msg);
    }
}
