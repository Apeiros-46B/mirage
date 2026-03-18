use std::sync::Arc;

use winit::{event::{ElementState, MouseButton}, keyboard::PhysicalKey, window::Window};

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
    pub win: Arc<Window>,
    pub app_tx: GameToApp,
}

pub enum GameMsg {
    Resized,
    KeyPress(PhysicalKey, ElementState),
    MouseClick(MouseButton, ElementState),
    MouseMotion(f32, f32),
}

channel_routing_for!(Game {
    primary: [GameMsg; 8192] blocking,
});

pub struct RendererInit {
    pub ctx: Arc<VulkanCtx>,
    pub win: Arc<Window>,
    pub app_tx: RendererToApp,
}

pub enum RendererMsg {
    Resized,
}

channel_routing_for!(Renderer {
    primary: [RendererMsg; 8192],
});
