use egui::{Color32, Context, Ui, Window};
use egui_plot::{HLine, Line, Plot, PlotBounds, PlotPoints, VLine};
use winit::event_loop::ActiveEventLoop;

use crate::{
    sync::worker::MessageHandler,
    util::HistoryRing,
    workers::msgs::{GAME_TICKRATE_HZ, GameTelemetry, RendererTelemetry},
};

pub const HISTORY_CAPACITY: usize = 256;

pub struct UIState {
    pub frame_secs: f32,
    pub frame_secs_hist: HistoryRing<f32>,
    pub tick_ms: f32,
    pub tick_ms_hist: HistoryRing<f32>,
    pub lag_ms: f32,
    pub lag_ms_hist: HistoryRing<f32>,
}

impl MessageHandler<GameTelemetry> for UIState {
    fn handle(&mut self, msg: GameTelemetry) {
        self.tick_ms = msg.tick_ms;
        self.tick_ms_hist.write(msg.tick_ms);
        self.lag_ms = msg.lag_ms;
        self.lag_ms_hist.write(msg.lag_ms);
    }
}

impl MessageHandler<RendererTelemetry> for UIState {
    fn handle(&mut self, msg: RendererTelemetry) {
        self.frame_secs = msg.frame_secs;
        self.frame_secs_hist.write(msg.frame_secs * 1000.0);
    }
}

impl Default for UIState {
    fn default() -> Self {
        Self {
            frame_secs: 0.0,
            frame_secs_hist: HistoryRing::new(HISTORY_CAPACITY, 0.0),
            tick_ms: 0.0,
            tick_ms_hist: HistoryRing::new(HISTORY_CAPACITY, 0.0),
            lag_ms: 0.0,
            lag_ms_hist: HistoryRing::new(HISTORY_CAPACITY, 0.0),
        }
    }
}

#[derive(Default)]
pub struct UIResponse {
    should_exit: bool,
}

impl UIResponse {
    pub fn handle(&self, evt_loop: &ActiveEventLoop) {
        if self.should_exit {
            evt_loop.exit();
        }
    }
}

pub fn ui_main(state: &UIState, resp: &mut UIResponse, ctx: &Context) {
    ctx.style_mut(|style| {
        style.visuals.window_shadow.offset = [0, 0];
        style.visuals.popup_shadow.offset = [0, 0];
    });

    Window::new("Timings").show(ctx, |ui| debug_window(state, resp, ui));
}

fn debug_window(state: &UIState, resp: &mut UIResponse, ui: &mut Ui) {
    let budget_ms = 1000.0 / GAME_TICKRATE_HZ as f32;
    let fps = 1.0 / state.frame_secs;

    ui.label(format!(
        "tick {:06.3}ms / budget {:06.3}ms / lag {:06.3}ms",
        state.tick_ms, budget_ms, state.lag_ms,
    ));
    sparkline(ui, "tick_compute_ms", &state.tick_ms_hist, Some(budget_ms));
    sparkline(ui, "tick_lag_ms", &state.lag_ms_hist, None);

    ui.label(format!("frame {:06.3}ms {fps:.0}/s", state.frame_secs * 1000.0));
    sparkline(ui, "frame_time_secs", &state.frame_secs_hist, None);

    if ui.button("Quit").clicked() {
        resp.should_exit = true;
    }
}

fn sparkline(ui: &mut Ui, id: &str, hist: &HistoryRing<f32>, budget: Option<f32>) {
    let points: PlotPoints = hist.iter_head(f32::NAN)
        .enumerate()
        .map(|(i, v)| [i as f64, v.into()])
        .collect();

    let data = Line::new(id, points)
        .color(Color32::from_rgb(100, 200, 100))
        .width(1.5_f32);

    let sweep = VLine::new(id, hist.head as f64)
        .color(Color32::from_rgba_unmultiplied(255, 0, 0, 200))
        .width(2.0_f32);

    Plot::new(id)
        .height(35.0)
        .show_axes([false, false])
        .show_grid([false, false])
        .allow_drag(false)
        .allow_zoom(false)
        .allow_scroll(false)
        .set_margin_fraction(egui::vec2(0.0, 0.0))
        .show(ui, |plot| {
            plot.set_plot_bounds(PlotBounds::from_min_max(
                [0.0, 0.0],
                [HISTORY_CAPACITY as f64, 20.0],
            ));
            if let Some(b) = budget {
                plot.hline(HLine::new("budget", b as f64)
                    .color(Color32::from_rgb(220, 80, 80))
                    .width(1.0_f32));
            }
            plot.line(data);
            plot.vline(sweep);
        });
}
