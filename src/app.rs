use std::{sync::Arc, time::{Duration, Instant}};

use triple_buffer::triple_buffer;
use winit::{
    application::ApplicationHandler,
    event::{DeviceEvent, DeviceId, WindowEvent},
    event_loop::{ActiveEventLoop, ControlFlow},
    raw_window_handle::{HasDisplayHandle, HasWindowHandle},
    window::{CursorGrabMode, Window, WindowAttributes, WindowId}
};

use crate::{
    gpu::ctx::VulkanCtx,
    sync::{
        chan::SafeSend,
        worker::{MessageHandler, WorkerExt, WorkerHandle},
    },
    ui::{UIResponse, UIState, ui_main},
    workers::{
        game::{Game, GameInit},
        msgs::{
            AppMsg, AppRxBundle, AppTxBundle, EguiUpdate, GameMsg, GameTelemetry, RendererMsg, RendererTelemetry
        },
        renderer::{Renderer, RendererInit},
    }
};

pub struct AppState {
    ui: UIState,
    win: Arc<Window>,
    egui: egui_winit::State,
}

pub struct App {
    state: Option<AppState>,

    egui_ctx: egui::Context,
    egui_pending_textures: egui::TexturesDelta,
    next_redraw: Instant,

    rx: Option<AppRxBundle>,
    game: WorkerHandle<Game>,
    renderer: WorkerHandle<Renderer>,

    // keep window alive
    _win: Option<Arc<Window>>,
}

impl App {
    pub fn new() -> Self {
        let egui_ctx = egui::Context::default();

        Self {
            state: None,

            egui_ctx,
            egui_pending_textures: Default::default(),
            next_redraw: Instant::now(),

            rx: None,
            game: Game::spawn(),
            renderer: Renderer::spawn(),

            _win: None,
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

        let (camera_tx, camera_rx) = triple_buffer(&Default::default());

        self.renderer.resume(RendererInit {
            sz: win.inner_size(),
            ctx: Arc::new(ctx),
            app_tx: tx.take_renderer().unwrap(),
            camera_rx,
        });

        self.game.resume(GameInit {
            sz: win.inner_size(),
            app_tx: tx.take_game().unwrap(),
            camera_tx,
        });
    }

    #[allow(dead_code)]
    fn suspend_workers(&mut self) {
        self.renderer.suspend();
        self.game.suspend();
    }

    fn redraw(&mut self, evt_loop: &ActiveEventLoop) {
        let Some(this) = &mut self.state else { return };

        let sz = this.win.inner_size();
        if sz.width == 0 || sz.height == 0 {
            // we are minimized, don't do anything
            return;
        }

        let raw_input = this.egui.take_egui_input(&this.win);

        let mut resp = UIResponse::default();
        let full_output = self.egui_ctx.run(raw_input, |ctx| {
            ui_main(&this.ui, &mut resp, ctx);
        });
        resp.handle(evt_loop);
        this.egui.handle_platform_output(
            &this.win,
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

impl ApplicationHandler for App {
    fn resumed(&mut self, evt_loop: &ActiveEventLoop) {
        if self.state.is_some() {
            return;
        }

        let win = Arc::new(evt_loop
            .create_window(WindowAttributes::default())
            .expect("failed to open window"),
        );
        let egui = egui_winit::State::new(
            self.egui_ctx.clone(),
            egui::ViewportId::ROOT,
            &win,
            Some(win.scale_factor() as f32),
            None,
            None,
        );

        self.state = Some(AppState {
            ui: Default::default(),
            win: win.clone(),
            egui,
        });
        self._win = Some(win.clone());
        self.resume_workers(win);
    }

    fn about_to_wait(&mut self, evt_loop: &ActiveEventLoop) {
        let Some(this) = &mut self.state else { return };

        // handle incoming messages
        if let Some(rx) = &mut self.rx {
            rx.drain_all(this);
        }

        // ~60Hz UI processing
        // check time in case we woke up early from events
        let now = Instant::now();
        if now >= self.next_redraw {
            this.win.request_redraw();
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
        let Some(this) = &mut self.state else { return };
        let egui_res = this.egui.on_window_event(&this.win, &evt);

        match evt {
            WindowEvent::CloseRequested => evt_loop.exit(),
            WindowEvent::RedrawRequested => self.redraw(evt_loop),
            WindowEvent::Resized(sz) => {
                if sz.width != 0 && sz.height != 0 {
                    self.game.tx.send(GameMsg::Resized(sz));
                }
                self.renderer.tx.send(RendererMsg::Resized(sz));
            },
            WindowEvent::CursorMoved { position, .. } => {
                if egui_res.consumed {
                    return;
                }
                if self.egui_ctx.wants_pointer_input() {
                    return;
                }
                if self.egui_ctx.is_pointer_over_area() {
                    return;
                }
                self.game.tx.send(GameMsg::CursorMotion(
                    position.x as f32,
                    position.y as f32
                ));
            }
            WindowEvent::MouseInput { state, button, .. } => {
                if egui_res.consumed {
                    return;
                }
                if self.egui_ctx.wants_pointer_input() {
                    return;
                }
                if self.egui_ctx.is_pointer_over_area() {
                    return;
                }
                self.game.tx.send(GameMsg::MouseClick(button, state));
            }
            WindowEvent::KeyboardInput { event: k_evt, .. } => {
                if egui_res.consumed || k_evt.repeat {
                    return;
                }
                if self.egui_ctx.wants_keyboard_input() {
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
                // block mouse motion if interacting with UI
                if self.egui_ctx.wants_pointer_input() {
                    return;
                }
                if self.egui_ctx.is_pointer_over_area() {
                    return;
                }

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

impl MessageHandler<AppMsg> for AppState {
    fn handle(&mut self, msg: AppMsg) {
        match msg {
            AppMsg::SetCursorCapture(captured) => {
                if captured {
                    self.win.set_cursor_visible(false);
                    // try strict lock with fallback
                    if self.win.set_cursor_grab(CursorGrabMode::Locked).is_err() {
                        let _ = self.win.set_cursor_grab(CursorGrabMode::Confined);
                    }
                } else {
                    self.win.set_cursor_visible(true);
                    let _ = self.win.set_cursor_grab(CursorGrabMode::None);
                }
            },
        }
    }
}

impl MessageHandler<GameTelemetry> for AppState {
    fn handle(&mut self, msg: GameTelemetry) {
        self.ui.handle(msg);
    }
}

impl MessageHandler<RendererTelemetry> for AppState {
    fn handle(&mut self, msg: RendererTelemetry) {
        self.ui.handle(msg);
    }
}
