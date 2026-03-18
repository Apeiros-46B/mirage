use crate::sync::chan::channel_routing_for;

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
