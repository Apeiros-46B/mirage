mod passes;
mod present;

use std::{sync::Arc, time::Duration};

use rtrb::{Consumer, Producer};
use winit::window::Window;

use crate::{
    gpu::{ctx::VulkanCtx, pass::Task},
    render::{
        passes::{ClearPass, FrameCtx, PresentPass, depth::DepthPrepass},
        present::Presenter,
    },
    sync::{chan::bounded, worker::Worker},
};

pub struct RenderInit {
    pub ctx: Arc<VulkanCtx>,
    pub win: Arc<Window>,
}

pub enum RenderMsg {
    Resized,
}

pub struct Renderer {
    presenter: Presenter,
    clear_pass: ClearPass,
    depth_prepass: DepthPrepass,
    present_pass: PresentPass,
    ctx: Arc<VulkanCtx>,
}

impl Worker for Renderer {
    const NAME: &'static str = "Renderer";

    type Init = RenderInit;
    type Msg = RenderMsg;
    type Tx = Producer<Self::Msg>;
    type Rx = Consumer<Self::Msg>;
    type AuxRx = ();

    fn channel() -> (Self::Tx, Self::Rx) {
        bounded(8192)
    }

    fn new(init: RenderInit) -> anyhow::Result<Self> {
        let ctx = init.ctx;
        let extent = crate::util::extent_of(&init.win);
        let presenter = Presenter::new(&ctx, init.win)?;

        let clear_pass = ClearPass;
        let depth_prepass = DepthPrepass::new(&ctx, extent)?;
        let present_pass = PresentPass;

        Ok(Self {
            ctx,
            presenter,
            clear_pass,
            depth_prepass,
            present_pass,
        })
    }

    fn tick(
        &mut self,
        elapsed: Duration,
        dt: Duration,
        tick: usize,
    ) -> anyhow::Result<()> {
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
            dt: dt.as_secs_f32(),
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

    fn handle(&mut self, msg: RenderMsg) {
        match msg {
            RenderMsg::Resized => self.presenter.resize_requested = true,
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
