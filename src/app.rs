use std::{sync::Arc, time::{Duration, Instant}};

use winit::{
    application::ApplicationHandler,
    event::{DeviceEvent, DeviceId, WindowEvent},
    event_loop::{ActiveEventLoop, ControlFlow},
    raw_window_handle::{HasDisplayHandle, HasWindowHandle},
    window::{Window, WindowAttributes, WindowId}
};

use crate::{
    gpu::ctx::VulkanCtx, sync::{
        chan::SafeSend,
        worker::{MessageHandler, WorkerExt, WorkerHandle},
    }, ui::{UIResponse, UIState, ui_main}, workers::{
        game::Game,
        msgs::{
            AppRxBundle,
            AppTxBundle,
            EguiUpdate,
            GameInit,
            GameMsg,
            GameTelemetry,
            RendererInit,
            RendererMsg,
            RendererTelemetry,
        },
        render::Renderer,
    }
};

pub struct App {
    initialised: bool,

    ui_state: UIState,
    egui_ctx: egui::Context,
    egui_state: Option<egui_winit::State>,
    egui_pending_textures: egui::TexturesDelta,
    next_redraw: Instant,

    win: Option<Arc<Window>>,

    rx: Option<AppRxBundle>,
    game: WorkerHandle<Game>,
    renderer: WorkerHandle<Renderer>,
}

impl App {
    pub fn new() -> Self {
        let egui_ctx = egui::Context::default();

        Self {
            initialised: false,

            ui_state: Default::default(),
            egui_ctx,
            egui_state: None,
            egui_pending_textures: Default::default(),
            next_redraw: Instant::now(),

            win: None,

            rx: None,
            game: Game::spawn(),
            renderer: Renderer::spawn(),
        }
    }

    fn resume_workers(&mut self, win: Arc<Window>) {
        let ctx = VulkanCtx::new(
                win.display_handle().expect("failed to get display").as_raw(),
                win.window_handle().expect("failed to get window").as_raw(),
                win.clone(),
            )
            .expect("failed to init Vulkan");

        let (mut tx, rx) = AppTxBundle::new_pair();
        self.rx = Some(rx);

        self.renderer.resume(RendererInit {
            sz: win.inner_size(),
            ctx: Arc::new(ctx),
            app_tx: tx.take_renderer().unwrap(),
        });

        self.game.resume(GameInit {
            sz: win.inner_size(),
            app_tx: tx.take_game().unwrap(),
        });
    }

    fn suspend_workers(&mut self) {
        self.renderer.suspend();
        self.game.suspend();
    }

    fn redraw(&mut self, evt_loop: &ActiveEventLoop) {
        let Some(win) = &self.win else { return };

        let sz = win.inner_size();
        if sz.width == 0 || sz.height == 0 {
            // we are minimized, don't do anything
            return;
        }

        let Some(egui_state) = &mut self.egui_state else { return };

        let raw_input = egui_state.take_egui_input(win);

        let mut resp = UIResponse::default();
        let full_output = self.egui_ctx.run(raw_input, |ctx| {
            ui_main(&self.ui_state, &mut resp, ctx);
        });
        resp.handle(evt_loop);
        egui_state.handle_platform_output(
            win,
            full_output.platform_output,
        );

        let clipped_primitives = self.egui_ctx.tessellate(
            full_output.shapes,
            full_output.pixels_per_point,
        );

        // aggregate texture deltas in case sending the update fails
        self.egui_pending_textures.append(full_output.textures_delta);

        let update = EguiUpdate {
            primitives: clipped_primitives,
            textures_delta: self.egui_pending_textures.clone(),
            px_per_pt: full_output.pixels_per_point,
            screen_sz_px: sz,
        };

        if self.renderer.aux_tx.app_egui.as_mut().expect("couldn't get egui queue").push(update).is_ok() {
            // only clear accumulated deltas if submission was successful
            self.egui_pending_textures.clear();
        }
    }
}

impl MessageHandler<GameTelemetry> for App {
    fn handle(&mut self, msg: GameTelemetry) {
        self.ui_state.tick_time = msg.tick_time;
    }
}

impl MessageHandler<RendererTelemetry> for App {
    fn handle(&mut self, msg: RendererTelemetry) {
        self.ui_state.frame_time = msg.frame_time;
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

        self.resume_workers(win);
    }

    fn about_to_wait(&mut self, evt_loop: &ActiveEventLoop) {
        // handle incoming messages
        if let Some(mut rx) = self.rx.take() {
            rx.drain_all(self);
            self.rx = Some(rx);
        }

        let now = Instant::now();
        if now >= self.next_redraw {
            if let Some(win) = self.win.as_ref() {
                win.request_redraw();
            }
            // ~60Hz UI processing
            self.next_redraw = now + Duration::from_secs_f64(1.0 / 60.0);
        }

        evt_loop.set_control_flow(ControlFlow::WaitUntil(self.next_redraw));
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
            WindowEvent::CloseRequested => evt_loop.exit(),
            WindowEvent::RedrawRequested => self.redraw(evt_loop),
            WindowEvent::Resized(sz) => {
                if sz.width != 0 && sz.height != 0 {
                    self.game.tx.send(GameMsg::Resized(sz));
                }
                self.renderer.tx.send(RendererMsg::Resized(sz));
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
