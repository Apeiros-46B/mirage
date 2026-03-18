use std::{sync::Arc, time::Duration};

use winit::{
    application::ApplicationHandler,
    event::{DeviceEvent, DeviceId, StartCause, WindowEvent},
    event_loop::{ActiveEventLoop, ControlFlow},
    raw_window_handle::{HasDisplayHandle, HasWindowHandle},
    window::{Window, WindowAttributes, WindowId},
};

use crate::{
    gpu::ctx::VulkanCtx,
    sync::{
        chan::{MessageHandler, RxBundle, SafeSend, TxBundle},
        worker::{WorkerExt, WorkerHandle},
    },
    workers::{
        game::{Game, GameInit, GameMsg},
        msgs::{AppRxBundle, AppTxBundle, GameTelemetry, RendererTelemetry},
        render::{Renderer, RendererInit, RendererMsg},
    },
};

pub struct App {
    initialised: bool,

    egui_ctx: egui::Context,
    egui_state: Option<egui_winit::State>,
    win: Option<Arc<Window>>,

    aux_tx: Option<AppTxBundle>,
    aux_rx: Option<AppRxBundle>,
    game: WorkerHandle<Game>,
    renderer: WorkerHandle<Renderer>,
}

impl App {
    pub fn new() -> Self {
        let egui_ctx = egui::Context::default();

        let (aux_tx, aux_rx) = TxBundle::<App>::new();

        Self {
            initialised: false,
            egui_ctx,
            egui_state: None,
            win: None,
            aux_tx: Some(aux_tx),
            aux_rx: Some(aux_rx),
            game: Game::spawn(),
            renderer: Renderer::spawn(),
        }
    }
}

impl MessageHandler<GameTelemetry> for App {
    fn handle(&mut self, msg: GameTelemetry) {
        dbg!(msg);
    }
}

impl MessageHandler<RendererTelemetry> for App {
    fn handle(&mut self, msg: RendererTelemetry) {
        dbg!(msg);
    }
}

impl ApplicationHandler for App {
    fn resumed(&mut self, evt_loop: &ActiveEventLoop) {
        if self.initialised {
            return;
        }
        self.initialised = true;

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

        self.renderer.resume(RendererInit {
            ctx: Arc::new(ctx),
            win: win.clone(),
            app_tx: aux_tx.take_renderer().unwrap(),
        });

        self.game.resume(GameInit {
            win,
            app_tx: aux_tx.take_game().unwrap(),
        });
    }

    fn about_to_wait(&mut self, evt_loop: &ActiveEventLoop) {
        // handle incoming messages
        if let Some(mut aux_rx) = self.aux_rx.take() {
            aux_rx.drain_all(self);
            self.aux_rx = Some(aux_rx);
        }

        // ~60Hz
        let dur = Duration::from_secs_f64(1.0 / 60.0);
        evt_loop.set_control_flow(ControlFlow::wait_duration(dur));
    }

    fn new_events(
        &mut self,
        _evt_loop: &ActiveEventLoop,
        cause: StartCause,
    ) {
        // redraw every time we wake up due to sleep timeout
        if let StartCause::ResumeTimeReached { .. } = cause {
            if let Some(win) = self.win.as_ref() {
                win.request_redraw();
            }
        }
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
                // TODO: egui
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
