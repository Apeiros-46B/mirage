use std::{sync::Arc};

use winit::{
    application::ApplicationHandler,
    event::{DeviceEvent, DeviceId, WindowEvent},
    event_loop::{ActiveEventLoop, ControlFlow},
    raw_window_handle::{HasDisplayHandle, HasWindowHandle},
    window::{Window, WindowAttributes, WindowId},
};

use crate::{
    game::{Game, GameInit, GameMsg},
    gpu::ctx::VulkanCtx,
    render::{RenderInit, RenderMsg, Renderer},
    sync::{Worker, WorkerHandle},
};

pub struct App {
    win: Option<Arc<Window>>,
    game: WorkerHandle<Game>,
    renderer: WorkerHandle<Renderer>,
}

impl App {
    pub fn new() -> Self {
        Self {
            win: None,
            game: Game::spawn(),
            renderer: Renderer::spawn(),
        }
    }
}

impl ApplicationHandler for App {
    // on desktop platforms, this is only called once
    fn resumed(&mut self, evt_loop: &ActiveEventLoop) {
        // TODO: do we even need this if our game logic is on a separate thread?
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
            win: win.clone(),
        });

        self.game.init(GameInit { win });
    }

    fn window_event(
        &mut self,
        evt_loop: &ActiveEventLoop,
        _win_id: WindowId,
        evt: WindowEvent,
    ) {
        match evt {
            WindowEvent::CloseRequested => {
                evt_loop.exit();
            }
            WindowEvent::Resized(_) => {
                self.game.send(GameMsg::Resized);
                self.renderer.send(RenderMsg::Resized);
            },
            WindowEvent::MouseInput { state, button, .. } => {
                self.game.send(GameMsg::MouseClick(button, state));
            }
            WindowEvent::KeyboardInput { event: k_evt, .. } => {
                if k_evt.repeat {
                    return;
                }
                self.game.send(GameMsg::KeyPress(k_evt.physical_key, k_evt.state));
            }
            _ => {}
        }
    }

    fn device_event(
        &mut self,
        _evt_loop: &ActiveEventLoop,
        _dev_id: DeviceId,
        evt: DeviceEvent,
    ) {
        match evt {
            DeviceEvent::MouseMotion { delta } => {
                let (dx, dy) = delta;
                self.game.send(GameMsg::MouseMotion(dx as f32, dy as f32));
            }
            _ => {}
        }
    }

    fn exiting(&mut self, _evt_loop: &ActiveEventLoop) {
        self.game.exit();
        self.renderer.exit();
    }
}
