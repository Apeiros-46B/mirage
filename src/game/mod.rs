use std::time::Instant;

use winit::{event::{DeviceEvent, WindowEvent}, window::Window};

use crate::game::{camera::Camera, input::InputState};

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

pub struct GameState {
    prev_t: Instant,
    input: InputState,
    camera: Camera,
}

impl GameState {
    pub fn new(win: &Window) -> Self {
        Self {
            prev_t: Instant::now(),
            input: InputState::new(),
            camera: Camera::new(win.inner_size()),
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

impl EventHandler for GameState {
    fn handle_win_evt(&mut self, evt: &WindowEvent) {
        self.input.handle_win_evt(evt);
        self.camera.handle_win_evt(evt);
    }

    fn handle_dev_evt(&mut self, evt: &DeviceEvent) {
        self.input.handle_dev_evt(evt);
    }
}
