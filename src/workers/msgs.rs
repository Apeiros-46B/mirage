#![allow(dead_code)]

use egui::{ClippedPrimitive, TexturesDelta};
use winit::{
    dpi::PhysicalSize,
    event::{ElementState, MouseButton},
    keyboard::PhysicalKey,
};

use crate::sync::worker::channel_routing_for;

pub enum AppMsg {
    SetCursorCapture(bool),
}

#[derive(Debug)]
pub struct GameTelemetry {
    pub tick_time: f32,
}

#[derive(Debug)]
pub struct RendererTelemetry {
    pub frame_time: f32,
}

channel_routing_for!(App {
    Game => {
        msgs: [AppMsg; 64],
        telemetry: [GameTelemetry; 64],
    },
    Renderer => {
        telemetry: [RendererTelemetry; 64],
    },
});

pub enum GameMsg {
    Resized(PhysicalSize<u32>),
    KeyPress(PhysicalKey, ElementState),
    MouseClick(MouseButton, ElementState),
    MouseMotion(f32, f32), // raw device motion, dx dy
    CursorMotion(f32, f32), // cursor position, x y
}

channel_routing_for!(Game {
    primary: [GameMsg; 8192] blocking,
});

pub enum RendererMsg {
    Resized(PhysicalSize<u32>),
}

#[derive(Debug)]
pub struct EguiUpdate {
    pub primitives: Vec<ClippedPrimitive>,
    pub textures_delta: TexturesDelta,
    pub px_per_pt: f32,
    pub screen_sz_px: PhysicalSize<u32>,
}

channel_routing_for!(Renderer {
    primary: [RendererMsg; 8192],
    App => {
        egui: [EguiUpdate; 64],
    }
});
