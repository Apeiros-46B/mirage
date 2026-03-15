mod passes;
mod present;

use std::{sync::Arc, time::Instant};

use ash::vk::Extent2D;
use rtrb::Consumer;
use winit::window::Window;

use crate::{
    gpu::{ctx::VulkanCtx, pass::Task},
    render::{passes::{ClearPass, FrameCtx, PresentPass, depth::DepthPrepass}, present::Presenter},
    sync::{OneshotRx, RenderInit, RenderMsg},
};

pub struct Renderer {
    presenter: Presenter,
    clear_pass: ClearPass,
    depth_prepass: DepthPrepass,
    present_pass: PresentPass,

    rx: Consumer<RenderMsg>,
    ctx: Arc<VulkanCtx>,
}

fn extent_of(win: &Window) -> Extent2D {
    let inner = win.inner_size();
    Extent2D { width: inner.width, height: inner.height }
}

impl Renderer {
    fn new(init: RenderInit, rx: Consumer<RenderMsg>) -> anyhow::Result<Self> {
        let ctx = init.ctx;
        let extent = extent_of(&init.win);
        let presenter = Presenter::new(&ctx, init.win)?;

        let clear_pass = ClearPass;
        let depth_prepass = DepthPrepass::new(&ctx, extent)?;
        let present_pass = PresentPass;

        Ok(Self {
            rx,
            ctx,
            presenter,
            clear_pass,
            depth_prepass,
            present_pass,
        })
    }

    fn draw_frame(&mut self, frame_idx: usize, time: f64, dt: f32) -> anyhow::Result<()> {
        let Some((target, cmd, did_resize)) = self.presenter.begin_frame(&self.ctx)? else {
            return Ok(()); // minimized or out-of-date, skip drawing
        };

        if did_resize {
            self.clear_pass.resize(&self.ctx, target.extent)?;
            self.present_pass.resize(&self.ctx, target.extent)?;
        }

        let frame = FrameCtx {
            frame_idx,
            time,
            dt,
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

    fn run(mut self) {
        let start_time = Instant::now();
        let mut prev_frame_time = start_time;
        let mut frame_idx = 0usize;

        loop {
            while let Ok(msg) = self.rx.pop() {
                match msg {
                    RenderMsg::Exiting => return,
                    RenderMsg::Resized => {
                        self.presenter.resize_requested = true;
                    }
                }
            }

            let now = Instant::now();
            let time = (now - start_time).as_secs_f64();
            let dt = (now - prev_frame_time).as_secs_f32();

            if let Err(e) = self.draw_frame(frame_idx, time, dt) {
                eprintln!("render err: {e}");
            }

            prev_frame_time = now;
            frame_idx += 1;
        }
    }

    pub fn start(init_rx: OneshotRx<RenderInit>, rx: Consumer<RenderMsg>) {
        // this call blocks until we receive necessary data from main
        // (window handles for surface creation, because on some platforms
        // you cannot extract the raw window handle on a non-main thread)
        let Some(init) = init_rx.recv() else { return };
        Self::new(init, rx).expect("failed to init renderer").run();
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
