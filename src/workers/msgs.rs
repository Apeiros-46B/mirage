#![allow(dead_code)]

use std::sync::Arc;

use egui::{ClippedPrimitive, TexturesDelta};
use winit::{
    dpi::PhysicalSize,
    event::{ElementState, MouseButton},
    keyboard::PhysicalKey,
};

use crate::{gpu::ctx::VulkanCtx, sync::worker::channel_routing_for};

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
        telemetry: [GameTelemetry; 64],
    },
    Renderer => {
        telemetry: [RendererTelemetry; 64],
    },
});

pub struct GameInit {
    pub sz: PhysicalSize<u32>,
    pub app_tx: GameToApp,
}

pub enum GameMsg {
    Resized(PhysicalSize<u32>),
    KeyPress(PhysicalKey, ElementState),
    MouseClick(MouseButton, ElementState),
    MouseMotion(f32, f32),
}

channel_routing_for!(Game {
    primary: [GameMsg; 8192] blocking,
});

pub struct RendererInit {
    pub sz: PhysicalSize<u32>,
    pub ctx: Arc<VulkanCtx>,
    pub app_tx: RendererToApp,
}

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
