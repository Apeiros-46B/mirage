pub mod camera;
pub mod input;

use std::{sync::Arc, time::Duration};

use triple_buffer::Input;
use winit::dpi::PhysicalSize;

use crate::{sync::{
    chan::SafeSend,
    worker::{MessageHandler, Worker, WorkerControl, WorkerExt},
}, workers::msgs::AppMsg};
use super::msgs::{GameMsg, GameRxBundle, GameTelemetry, GameToApp};
use camera::{Camera, ThinCamera};
use input::InputState;

pub trait Polled {
    fn poll(&mut self, _input: &mut InputState, _dt: f32) {}
    fn poll_post(&mut self) {}
}

pub struct GameInit {
    pub sz: PhysicalSize<u32>,
    pub app_tx: GameToApp,
    pub camera_tx: Input<ThinCamera>,
}

pub struct GameState {
    input: InputState,
    camera: Camera,
    app_tx: GameToApp,
    camera_tx: Input<ThinCamera>
}

#[derive(Default)]
pub struct Game;

impl Worker for Game {
    type Init = GameInit;
    type State = GameState;
    type Channels = GameRxBundle;

    fn on_resume(&self, init: Self::Init) -> anyhow::Result<Self::State> {
        Ok(GameState {
            input: InputState::new(),
            camera: Camera::new(init.sz),
            app_tx: init.app_tx,
            camera_tx: init.camera_tx,
        })
    }

    fn on_tick(
        this: &mut Self::State,
        _since_wake: Duration,
        since_tick: Duration,
        _tick: usize,
    ) -> anyhow::Result<()> {
        let dt = since_tick.as_secs_f32();

        this.app_tx.telemetry.send(GameTelemetry { tick_time: dt });
        if let Some(captured) = this.input.take_cursor_capture() {
            this.app_tx.msgs.send(AppMsg::SetCursorCapture(captured));
        }

        this.camera.poll(&mut this.input, dt);
        this.input.reset_keys();

        this.camera_tx.write(this.camera.thin());

        Ok(())
    }

    fn run(self, ctrl: Arc<WorkerControl<Self::Init>>, rx: Self::Channels) {
        self.run_tickrate(ctrl, rx, 120.0);
    }
}

impl MessageHandler<GameMsg> for GameState {
    fn handle(&mut self, msg: GameMsg) {
        match msg {
            GameMsg::Resized(sz) => self.camera.resize(sz),
            GameMsg::KeyPress(key, state) => self.input.handle_input(key, state),
            GameMsg::MouseClick(btn, state) => self.input.handle_input(btn, state),
            GameMsg::MouseMotion(dx, dy) => {
                self.input.handle_mouse(dx, dy);
                self.camera.poll(&mut self.input, 0.0);
                // ^ 0.0 dt to apply mouse movements only
            },
            GameMsg::CursorMotion(x, y) => self.input.handle_cursor(x, y),
        }
    }
}
