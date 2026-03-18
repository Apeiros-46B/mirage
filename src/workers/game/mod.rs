pub mod camera;
pub mod input;

use std::{sync::{Arc, atomic::AtomicBool}, time::Duration};

use winit::{event::{ElementState, MouseButton}, keyboard::PhysicalKey, window::Window};

use crate::sync::{
    chan::{ConsumerBlocking, ProducerBlocking, SafeSend, bounded_deadline},
    worker::{Worker, WorkerExt},
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

pub struct Game {
    input: InputState,
    camera: Camera,
    win: Arc<Window>,
    app_tx: GameToApp,
}

impl Worker for Game {
    type Init = GameInit;
    type Msg = GameMsg;
    type Tx = ProducerBlocking<Self::Msg>;
    type Rx = ConsumerBlocking<Self::Msg>;

    type AuxTx = ();
    type AuxRx = ();

    fn channel() -> (Self::Tx, Self::Rx) {
        bounded_deadline(8192)
    }

    fn new(init: Self::Init) -> anyhow::Result<Self> {
        Ok(Self {
            input: InputState::new(),
            camera: Camera::new(init.win.inner_size()),
            win: init.win,
            app_tx: init.app_tx,
        })
    }

    fn tick(
        &mut self,
        _elapsed: Duration,
        dt: Duration,
        _tick: usize,
    ) -> anyhow::Result<()> {
        let dt_f = dt.as_secs_f32();

        self.app_tx.telemetry.send(GameTelemetry { tick_time: dt_f });

        self.camera.poll(&mut self.input, dt_f);
        self.input.reset_keys();

        Ok(())
    }

    fn run(
        self,
        rx: Self::Rx,
        aux_rx: Self::AuxRx,
        should_exit: Arc<AtomicBool>,
    ) {
        self.run_tickrate(rx, aux_rx, should_exit, 120.0);
    }

    fn handle(&mut self, msg: Self::Msg) {
        match msg {
            GameMsg::KeyPress(key, state) => self.input.handle_input(key, state),
            GameMsg::MouseClick(btn, state) => self.input.handle_input(btn, state),
            GameMsg::MouseMotion(dx, dy) => {
                self.input.handle_mouse(dx, dy);
                self.camera.poll(&mut self.input, 0.0);
                // ^ 0.0 dt to apply mouse movements only
            },
            GameMsg::Resized => self.camera.resize(self.win.inner_size()),
        }
    }
}
