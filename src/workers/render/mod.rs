mod passes;
mod present;

use std::{sync::Arc, time::Duration};

use rtrb::{Consumer, Producer};
use winit::window::Window;

use crate::{
    gpu::{ctx::VulkanCtx, pass::Task},
    sync::{chan::{SafeSend, bounded}, worker::Worker},
};
use super::msgs::{RendererTelemetry, RendererToApp};
use passes::{depth::DepthPrepass, frame::{ClearPass, FrameCtx, PresentPass}};
use present::Presenter;

pub struct RendererInit {
    pub ctx: Arc<VulkanCtx>,
    pub win: Arc<Window>,
    pub app_tx: RendererToApp,
}

pub enum RendererMsg {
    Resized,
}

pub struct Renderer {
    presenter: Presenter,
    clear_pass: ClearPass,
    depth_prepass: DepthPrepass,
    present_pass: PresentPass,
    app_tx: RendererToApp,
    ctx: Arc<VulkanCtx>,
}

impl Worker for Renderer {
    type Init = RendererInit;
    type Msg = RendererMsg;
    type Tx = Producer<Self::Msg>;
    type Rx = Consumer<Self::Msg>;

    type AuxTx = ();
    type AuxRx = ();

    fn channel() -> (Self::Tx, Self::Rx) {
        bounded(8192)
    }

    fn new(init: RendererInit) -> anyhow::Result<Self> {
        let ctx = init.ctx;
        let app_tx = init.app_tx;
        let extent = crate::util::extent_of(&init.win);
        let presenter = Presenter::new(&ctx, init.win)?;

        let clear_pass = ClearPass;
        let depth_prepass = DepthPrepass::new(&ctx, extent)?;
        let present_pass = PresentPass;

        Ok(Self {
            presenter,
            clear_pass,
            depth_prepass,
            present_pass,
            app_tx,
            ctx,
        })
    }

    fn tick(
        &mut self,
        elapsed: Duration,
        dt: Duration,
        tick: usize,
    ) -> anyhow::Result<()> {
        let dt_f = dt.as_secs_f32();

        self.app_tx.telemetry.send(RendererTelemetry { frame_time: dt_f });

        let Some((target, cmd, resize)) = self.presenter.begin_frame(&self.ctx)? else {
            return Ok(()); // minimized or out-of-date, skip drawing
        };

        if resize {
            self.clear_pass.resize(&self.ctx, target.extent)?;
            self.present_pass.resize(&self.ctx, target.extent)?;
            self.depth_prepass.resize(&self.ctx, target.extent)?;
        }

        let frame = FrameCtx {
            frame_idx: tick,
            time: elapsed.as_secs_f64(),
            dt: dt_f,
            target,
            swapchain_base_barrier: self.presenter.swapchain.base_barrier(),
        };

        unsafe {
            self.clear_pass.execute(&self.ctx, &frame, cmd)?;
            self.depth_prepass.execute(&self.ctx, &frame, cmd)?;
            self.present_pass.execute(&self.ctx, &frame, cmd)?;
        }

        self.presenter.submit_frame(&self.ctx)
    }

    fn handle(&mut self, msg: RendererMsg) {
        match msg {
            RendererMsg::Resized => self.presenter.resize_requested = true,
        }
    }
}

impl Drop for Renderer {
    fn drop(&mut self) {
        // ctx is dropped by the compiler after this is done
        unsafe {
            let _ = self.ctx.dev.device_wait_idle();
            self.presenter.destroy(&self.ctx);
        }
    }
}
