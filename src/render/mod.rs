mod present;

use rtrb::Consumer;

use crate::{
    gpu::ctx::VulkanCtx,
    render::present::Presenter,
    sync::{OneshotRx, RenderInit, RenderMsg},
};

pub struct Renderer {
    rx: Consumer<RenderMsg>,
    presenter: Presenter,
    ctx: VulkanCtx, // should be dropped last
}

impl Renderer {
    fn new(init: RenderInit, rx: Consumer<RenderMsg>) -> anyhow::Result<Self> {
        let ctx = VulkanCtx::new(init.raw_disp, init.raw_win, init.win.clone())?;
        let presenter = Presenter::new(&ctx, init.win)?;
        Ok(Self { rx, ctx, presenter })
    }

    fn draw_frame(&mut self) -> anyhow::Result<()> {
        let Some((target, cmd)) = self.presenter.begin_frame(&self.ctx)? else {
            return Ok(()); // minimized or out-of-date, skip drawing
        };

        unsafe {
            // TODO: clear the screen to a color between black and white
            // based on the sine of the current time (oscillate once every second)
        }

        self.presenter.submit_frame(&self.ctx)
    }

    fn run(mut self) {
        loop {
            while let Ok(msg) = self.rx.pop() {
                match msg {
                    RenderMsg::Exiting => return,
                    RenderMsg::Resized => {
                        self.presenter.resize_requested = true;
                    }
                }
            }

            if let Err(e) = self.draw_frame() {
                eprintln!("render err: {e}");
            }
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
        unsafe {
            let _ = self.ctx.dev.device_wait_idle();
            self.presenter.destroy(&self.ctx);
        }
    }
}
