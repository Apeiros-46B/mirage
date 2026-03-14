use std::{sync::Arc, thread::{self, JoinHandle}};

use rtrb::{Producer, RingBuffer};
use winit::{
    application::ApplicationHandler,
    event::WindowEvent,
    event_loop::ActiveEventLoop,
    raw_window_handle::{HasDisplayHandle, HasWindowHandle},
    window::{Window, WindowAttributes, WindowId},
};

use crate::{
    gpu::ctx::VulkanCtx, render::Renderer, sync::{OneshotTx, RenderInit, RenderMsg, oneshot}
};

pub struct App {
    win: Option<Arc<Window>>,
    render_thread: Option<JoinHandle<()>>,
    init_tx: Option<OneshotTx<RenderInit>>,
    tx: Producer<RenderMsg>,
}

impl App {
    pub fn new() -> Self {
        let (init_tx, init_rx) = oneshot();
        let (tx, rx) = RingBuffer::new(256);
        let render_thread = thread::spawn(move || Renderer::start(init_rx, rx));
        Self {
            win: None,
            render_thread: Some(render_thread),
            init_tx: Some(init_tx),
            tx,
        }
    }
}

impl ApplicationHandler for App {
    // on desktop platforms, this is only called once
    fn resumed(&mut self, evt_loop: &ActiveEventLoop) {
        let win = Arc::new(evt_loop
            .create_window(WindowAttributes::default())
            .expect("failed to open window"),
        );
        self.win = Some(win.clone());

        let ctx = VulkanCtx::new(
                win.display_handle().expect("failed to get display").as_raw(),
                win.window_handle().expect("failed to get window").as_raw(),
                win.clone(),
            )
            .expect("failed to init vulkan");

        if let Some(init_tx) = self.init_tx.take() {
            init_tx.send(RenderInit {
                ctx: Arc::new(ctx),
                win,
            });
        }
    }

    fn window_event(
        &mut self,
        evt_loop: &ActiveEventLoop,
        winid: WindowId,
        evt: WindowEvent,
    ) {
        match evt {
            WindowEvent::Resized(_) => {
                self.tx.push(RenderMsg::Resized).ok();
            },
            WindowEvent::CloseRequested => {
                evt_loop.exit();
            }
            _ => {},
        }
    }

    fn exiting(&mut self, evt_loop: &ActiveEventLoop) {
        while self.tx.push(RenderMsg::Exiting).is_err() {
            std::thread::yield_now();
        }
        self.render_thread.take().unwrap().join().ok();
    }
}
