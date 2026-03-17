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
    sync::{chan::SafeSend, worker::{WorkerExt, WorkerHandle}},
};

pub struct App {
    win: Option<Arc<Window>>,

    // egui_ctx: egui::Context,
    // egui_state: Option<egui_winit::State>,

    game: WorkerHandle<Game>,
    renderer: WorkerHandle<Renderer>,
}

impl App {
    pub fn new() -> Self {
        // let egui_ctx = egui::Context::default();

        Self {
            win: None,
            // egui_ctx,
            // egui_state: None,
            game: Game::spawn(),
            renderer: Renderer::spawn(),
        }
    }
}

impl ApplicationHandler for App {
    // on desktop platforms, this is only called once
    fn resumed(&mut self, evt_loop: &ActiveEventLoop) {
        evt_loop.set_control_flow(ControlFlow::Wait);

        let win = Arc::new(evt_loop
            .create_window(WindowAttributes::default())
            .expect("failed to open window"),
        );
        self.win = Some(win.clone());
        // self.egui_state = Some(egui_winit::State::new(
        //     self.egui_ctx.clone(),
        //     egui::ViewportId::ROOT,
        //     &win,
        //     Some(win.scale_factor() as f32),
        //     None,
        //     None,
        // ));

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
        let Some(win) = self.win.as_ref() else {
            return;
        };

        let mut consumed = false;
        // if let Some(egui_state) = &mut self.egui_state {
        //     let res = egui_state.on_window_event(win, &evt);
        //     consumed = res.consumed;
        // }

        match evt {
            WindowEvent::CloseRequested => {
                evt_loop.exit();
            }
            WindowEvent::Resized(_) => {
                self.game.tx.send(GameMsg::Resized);
                self.renderer.tx.send(RenderMsg::Resized);
            },
            WindowEvent::MouseInput { state, button, .. } => {
                if consumed {
                    return;
                }
                self.game.tx.send(GameMsg::MouseClick(button, state));
            }
            WindowEvent::KeyboardInput { event: k_evt, .. } => {
                if consumed || k_evt.repeat {
                    return;
                }
                self.game.tx.send(GameMsg::KeyPress(k_evt.physical_key, k_evt.state));
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
                self.game.tx.send(GameMsg::MouseMotion(dx as f32, dy as f32));
            }
            _ => {}
        }
    }

    fn exiting(&mut self, _evt_loop: &ActiveEventLoop) {
        self.game.exit();
        self.renderer.exit();
    }
}
