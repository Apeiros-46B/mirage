pub mod camera;
pub mod input;

use std::sync::Arc;
use winit::{event::{ElementState, MouseButton}, keyboard::PhysicalKey, window::Window};

use crate::{game::{camera::Camera, input::InputState}, sync::Worker};

pub trait Polled {
    fn poll(&mut self, _input: &mut InputState, _dt: f32) {}
    fn poll_post(&mut self) {}
}

pub struct GameInit {
    pub win: Arc<Window>,
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
}

impl Worker for Game {
    const NAME: &'static str = "Game";

    type Init = GameInit;
    type Msg = GameMsg;

    fn new(init: Self::Init) -> anyhow::Result<Self> {
        Ok(Self {
            input: InputState::new(),
            camera: Camera::new(init.win.inner_size()),
            win: init.win,
        })
    }

    fn poll(&mut self, _time: f64, dt: f32, _tick_idx: usize) -> anyhow::Result<()> {
        // TODO: currently this is just a busy wait. should probably have some kind of timing/tick loop
        self.camera.poll(&mut self.input, dt);
        self.input.reset_keys();
        Ok(())
    }

    fn handle(&mut self, msg: Self::Msg) {
        match msg {
            GameMsg::KeyPress(key, state) => self.input.handle_input(key, state),
            GameMsg::MouseClick(btn, state) => self.input.handle_input(btn, state),
            GameMsg::MouseMotion(dx, dy) => self.input.handle_mouse(dx, dy),
            GameMsg::Resized => self.camera.resize(self.win.inner_size()),
        }
    }
}
