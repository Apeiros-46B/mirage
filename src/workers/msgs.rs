#![allow(dead_code)]

use bytemuck::{Pod, Zeroable};
use egui::{ClippedPrimitive, TexturesDelta};
use glam::{IVec3, Mat4, Vec3};
use winit::{
    dpi::PhysicalSize,
    event::{ElementState, MouseButton},
    keyboard::PhysicalKey,
};

use crate::sync::worker::channel_routing_for;

// matches shared/camera.slang::CameraState
#[derive(Clone, Copy, Debug, Default, Pod, Zeroable)]
#[repr(C)]
pub struct CameraState {
    pub rel_view_proj: Mat4,
    pub inv_proj: Mat4,
    pub inv_view: Mat4,
    pub chunk_pos: IVec3,
    pub _chunk_pad: i32,
    pub local_pos: Vec3,
    pub _local_pad: f32,
}

pub enum AppMsg {
    SetCursorCapture(bool),
}

/// tickrate of the game loop
pub const GAME_TICKRATE_HZ: f64 = 120.0;

#[derive(Debug)]
pub struct GameTelemetry {
    /// how long a tick's computation actually took
    pub tick_ms: f32,
    pub lag_ms: f32,
}

#[derive(Debug)]
pub struct RendererTelemetry {
    pub frame_secs: f32,
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
