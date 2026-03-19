pub mod camera;
pub mod input;

use std::{sync::Arc, time::Duration};

use crate::sync::{
    chan::SafeSend,
    worker::{MessageHandler, Worker, WorkerControl, WorkerExt},
};
use super::msgs::{GameInit, GameMsg, GameRxBundle, GameTelemetry, GameToApp};
use camera::Camera;
use input::InputState;

pub trait Polled {
    fn poll(&mut self, _input: &mut InputState, _dt: f32) {}
    fn poll_post(&mut self) {}
}

pub struct GameState {
    input: InputState,
    camera: Camera,
    app_tx: GameToApp,
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

        this.camera.poll(&mut this.input, dt);
        this.input.reset_keys();

        Ok(())
    }

    fn run(self, ctrl: Arc<WorkerControl<Self::Init>>, rx: Self::Channels) {
        self.run_tickrate(ctrl, rx, 120.0);
    }
}

impl MessageHandler<GameMsg> for GameState {
    fn handle(&mut self, msg: GameMsg) {
        match msg {
            GameMsg::KeyPress(key, state) => self.input.handle_input(key, state),
            GameMsg::MouseClick(btn, state) => self.input.handle_input(btn, state),
            GameMsg::MouseMotion(dx, dy) => {
                self.input.handle_mouse(dx, dy);
                self.camera.poll(&mut self.input, 0.0);
                // ^ 0.0 dt to apply mouse movements only
            },
            GameMsg::Resized(sz) => self.camera.resize(sz),
        }
    }
}
