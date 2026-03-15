use std::{sync::Arc};

use winit::{
    application::ApplicationHandler,
    event::WindowEvent,
    event_loop::{ActiveEventLoop, ControlFlow},
    raw_window_handle::{HasDisplayHandle, HasWindowHandle},
    window::{Window, WindowAttributes, WindowId},
};

use crate::{
    gpu::ctx::VulkanCtx,
    render::Renderer,
    sync::{RenderInit, RenderMsg, Worker, WorkerHandle},
};

pub struct App {
    win: Option<Arc<Window>>,
    renderer: WorkerHandle<Renderer>,
}

impl App {
    pub fn new() -> Self {
        Self {
            win: None,
            renderer: Renderer::spawn(),
        }
    }
}

impl ApplicationHandler for App {
    // on desktop platforms, this is only called once
    fn resumed(&mut self, evt_loop: &ActiveEventLoop) {
        evt_loop.set_control_flow(ControlFlow::Poll);

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
            .expect("failed to init Vulkan");

        self.renderer.init(RenderInit {
            ctx: Arc::new(ctx),
            win,
        });
    }

    fn window_event(
        &mut self,
        evt_loop: &ActiveEventLoop,
        _win_id: WindowId,
        evt: WindowEvent,
    ) {
        match evt {
            WindowEvent::Resized(_) => {
                self.renderer.send(RenderMsg::Resized);
            },
            WindowEvent::CloseRequested => {
                evt_loop.exit();
            }
            _ => {}
        }
    }

    fn exiting(&mut self, _evt_loop: &ActiveEventLoop) {
        self.renderer.exit();
    }
}
