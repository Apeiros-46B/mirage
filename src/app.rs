use std::{sync::Arc, time::Duration};

use winit::{
    application::ApplicationHandler,
    event::{DeviceEvent, DeviceId, WindowEvent},
    event_loop::{ActiveEventLoop, ControlFlow},
    raw_window_handle::{HasDisplayHandle, HasWindowHandle},
    window::{Window, WindowAttributes, WindowId},
};

use crate::{
    debug::{GameTelemetry, RendererTelemetry}, game::{Game, GameInit, GameMsg}, gpu::ctx::VulkanCtx, inbound_aux_channels, render::{Renderer, RendererInit, RendererMsg}, sync::{chan::SafeSend, worker::{AuxRx, AuxTx, WorkerExt, WorkerHandle}}
};

inbound_aux_channels!(App, {
    Game => {
        game_telemetry: [GameTelemetry; 64] => handle_game_telemetry,
    },
    Renderer => {
        renderer_telemetry: [RendererTelemetry; 64] => handle_renderer_telemetry,
    },
});

pub struct App {
    egui_ctx: egui::Context,
    egui_state: Option<egui_winit::State>,
    win: Option<Arc<Window>>,

    aux_tx: Option<AppAuxTx>,
    aux_rx: Option<AppAuxRx>,
    game: WorkerHandle<Game>,
    renderer: WorkerHandle<Renderer>,
}

impl App {
    pub fn new() -> Self {
        let egui_ctx = egui::Context::default();

        let (aux_tx, aux_rx) = AppAuxTx::new();

        Self {
            egui_ctx,
            egui_state: None,
            win: None,
            aux_tx: Some(aux_tx),
            aux_rx: Some(aux_rx),
            game: Game::spawn(),
            renderer: Renderer::spawn(),
        }
    }

    fn handle_game_telemetry(&mut self, msg: GameTelemetry) {
        // TODO
    }

    fn handle_renderer_telemetry(&mut self, msg: RendererTelemetry) {
        // TODO
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
        self.egui_state = Some(egui_winit::State::new(
            self.egui_ctx.clone(),
            egui::ViewportId::ROOT,
            &win,
            Some(win.scale_factor() as f32),
            None,
            None,
        ));

        let ctx = VulkanCtx::new(
                win.display_handle().expect("failed to get display").as_raw(),
                win.window_handle().expect("failed to get window").as_raw(),
                win.clone(),
            )
            .expect("failed to init Vulkan");

        let mut aux_tx = self.aux_tx.take().expect("failed to get aux_tx");

        self.renderer.init(RendererInit {
            ctx: Arc::new(ctx),
            win: win.clone(),
            telemetry_tx: aux_tx.take_renderer().unwrap().renderer_telemetry,
        });

        self.game.init(GameInit {
            win,
            telemetry_tx: aux_tx.take_game().unwrap().game_telemetry,
        });
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
        if let Some(egui_state) = &mut self.egui_state {
            let res = egui_state.on_window_event(win, &evt);
            consumed = res.consumed;
        }

        match evt {
            WindowEvent::CloseRequested => {
                evt_loop.exit();
            }
            WindowEvent::RedrawRequested => {
                if let Some(mut aux_rx) = self.aux_rx.take() {
                    aux_rx.drain_all(self);
                    self.aux_rx = Some(aux_rx);
                }
            }
            WindowEvent::Resized(_) => {
                self.game.tx.send(GameMsg::Resized);
                self.renderer.tx.send(RendererMsg::Resized);
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

        self.win.as_ref().unwrap().request_redraw();
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
