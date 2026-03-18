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

pub struct RendererState {
    presenter: Presenter,
    clear_pass: ClearPass,
    depth_prepass: DepthPrepass,
    present_pass: PresentPass,
    app_tx: RendererToApp,
    ctx: Arc<VulkanCtx>,
}

impl Drop for RendererState {
    fn drop(&mut self) {
        // ctx is dropped by the compiler after this is done
        unsafe {
            let _ = self.ctx.dev.device_wait_idle();
            self.presenter.destroy(&self.ctx);
        }
    }
}

#[derive(Default)]
pub struct Renderer;

impl Worker for Renderer {
    type State = RendererState;
    type Init = RendererInit;
    type Msg = RendererMsg;

    type Tx = Producer<Self::Msg>;
    type Rx = Consumer<Self::Msg>;

    type AuxTx = ();
    type AuxRx = ();

    fn channel() -> (Self::Tx, Self::Rx) {
        bounded(8192)
    }

    fn on_resume(&self, init: Self::Init) -> anyhow::Result<Self::State> {
        let ctx = init.ctx;
        let app_tx = init.app_tx;
        let extent = crate::util::extent_of(&init.win);
        let presenter = Presenter::new(&ctx, init.win)?;

        let clear_pass = ClearPass;
        let depth_prepass = DepthPrepass::new(&ctx, extent)?;
        let present_pass = PresentPass;

        Ok(RendererState {
            presenter,
            clear_pass,
            depth_prepass,
            present_pass,
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

        let Some((target, cmd, resize)) = this.presenter.begin_frame(&this.ctx)? else {
            return Ok(()); // minimized or out-of-date, skip drawing
        };

        if resize {
            this.clear_pass.resize(&this.ctx, target.extent)?;
            this.present_pass.resize(&this.ctx, target.extent)?;
            this.depth_prepass.resize(&this.ctx, target.extent)?;
        }

        let frame = FrameCtx {
            frame_idx: tick,
            time: since_wake.as_secs_f64(),
            dt,
            target,
            swapchain_base_barrier: this.presenter.swapchain.base_barrier(),
        };

        unsafe {
            this.clear_pass.execute(&this.ctx, &frame, cmd)?;
            this.depth_prepass.execute(&this.ctx, &frame, cmd)?;
            this.present_pass.execute(&this.ctx, &frame, cmd)?;
        }

        this.presenter.submit_frame(&this.ctx)
    }

    fn on_msg(this: &mut Self::State, msg: RendererMsg) {
        match msg {
            RendererMsg::Resized => this.presenter.resize_requested = true,
        }
    }
}
