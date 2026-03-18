pub mod camera;
pub mod input;

use std::{sync::Arc, time::Duration};

use winit::{event::{ElementState, MouseButton}, keyboard::PhysicalKey, window::Window};

use crate::sync::{
    chan::{ConsumerBlocking, ProducerBlocking, SafeSend, bounded_deadline},
    worker::{Worker, WorkerControl, WorkerExt},
};
use super::msgs::{GameTelemetry, GameToApp};
use camera::Camera;
use input::InputState;

pub trait Polled {
    fn poll(&mut self, _input: &mut InputState, _dt: f32) {}
    fn poll_post(&mut self) {}
}

pub struct GameInit {
    pub win: Arc<Window>,
    pub app_tx: GameToApp,
}

pub enum GameMsg {
    Resized,
    KeyPress(PhysicalKey, ElementState),
    MouseClick(MouseButton, ElementState),
    MouseMotion(f32, f32),
}

pub struct GameState {
    input: InputState,
    camera: Camera,
    win: Arc<Window>,
    app_tx: GameToApp,
}

#[derive(Default)]
pub struct Game;

impl Worker for Game {
    type State = GameState;
    type Init = GameInit;
    type Msg = GameMsg;

    type Tx = ProducerBlocking<Self::Msg>;
    type Rx = ConsumerBlocking<Self::Msg>;

    type AuxTx = ();
    type AuxRx = ();

    fn channel() -> (Self::Tx, Self::Rx) {
        bounded_deadline(8192)
    }

    fn on_resume(&self, init: Self::Init) -> anyhow::Result<Self::State> {
        Ok(GameState {
            input: InputState::new(),
            camera: Camera::new(init.win.inner_size()),
            win: init.win,
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

    fn on_msg(this: &mut Self::State, msg: Self::Msg) {
        match msg {
            GameMsg::KeyPress(key, state) => this.input.handle_input(key, state),
            GameMsg::MouseClick(btn, state) => this.input.handle_input(btn, state),
            GameMsg::MouseMotion(dx, dy) => {
                this.input.handle_mouse(dx, dy);
                this.camera.poll(&mut this.input, 0.0);
                // ^ 0.0 dt to apply mouse movements only
            },
            GameMsg::Resized => this.camera.resize(this.win.inner_size()),
        }
    }

    fn run(
        self,
        ctrl: Arc<WorkerControl<Self::Init>>,
        rx: Self::Rx,
        aux_rx: Self::AuxRx,
    ) {
        self.run_tickrate(ctrl, rx, aux_rx, 120.0);
    }
}
