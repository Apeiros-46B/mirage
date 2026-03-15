use std::{sync::Arc, time::Instant};

use rtrb::Consumer;
use winit::{event::{DeviceEvent, WindowEvent}, window::Window};

use crate::{game::{camera::Camera, input::InputState}, sync::{GameInit, GameMsg, Worker}};

pub mod camera;
pub mod input;

pub trait Polled {
    fn poll(&mut self, _input: &mut InputState, _dt: f32) {}
    fn poll_post(&mut self) {}
}

pub trait EventHandler {
    fn handle_win_evt(&mut self, _evt: &WindowEvent) {}
    fn handle_dev_evt(&mut self, _evt: &DeviceEvent) {}
}

pub struct Game {
    prev_t: Instant,
    input: InputState,
    camera: Camera,
    win: Arc<Window>,
    rx: Consumer<GameMsg>,
}

impl Game {
    pub fn new(init: GameInit, rx: Consumer<GameMsg>) -> Self {
        Self {
            prev_t: Instant::now(),
            input: InputState::new(),
            camera: Camera::new(init.win.inner_size()),
            win: init.win,
            rx,
        }
    }

    pub fn update(&mut self) {
        let now = Instant::now();
        let dt = (now - self.prev_t).as_secs_f32();

        self.camera.poll(&mut self.input, dt);
        self.input.poll_post();

        self.prev_t = now;
    }
}

impl EventHandler for Game {
    fn handle_win_evt(&mut self, evt: &WindowEvent) {
        self.input.handle_win_evt(evt);
        self.camera.handle_win_evt(evt);
    }

    fn handle_dev_evt(&mut self, evt: &DeviceEvent) {
        self.input.handle_dev_evt(evt);
    }
}

impl Worker for Game {
    const NAME: &'static str = "Game";

    type Init = GameInit;
    type Msg = GameMsg;

    fn new(init: Self::Init) -> anyhow::Result<Self> {
        todo!()
    }

    fn poll(&mut self, time: f64, dt: f32, tick_idx: usize) -> anyhow::Result<()> {
        todo!()
    }

    fn handle(&mut self, msg: Self::Msg) {
        todo!()
    }
}
