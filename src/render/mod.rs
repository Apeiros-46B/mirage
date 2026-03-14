mod ctx;
mod surf;
mod swapchain;

use rtrb::Consumer;

use crate::{render::ctx::VulkanCtx, sync::{OneshotRx, RenderInit, RenderMsg}};

pub struct Renderer {
    rx: Consumer<RenderMsg>,
    ctx: VulkanCtx,
}

impl Renderer {
    fn new(init: RenderInit, rx: Consumer<RenderMsg>) -> anyhow::Result<Self> {
        let ctx = VulkanCtx::new(init)?;
        Ok(Self { rx, ctx })
    }

    fn run(mut self) {
        loop {
            let msg = self.rx.pop();
            if let Ok(RenderMsg::Exiting) = msg {
                break;
            }

            // TODO
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
