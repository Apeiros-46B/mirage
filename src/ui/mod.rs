use egui::{Color32, Context, Ui, Window};
use egui_plot::{Line, Plot, PlotBounds, PlotPoints, VLine};
use winit::event_loop::ActiveEventLoop;

use crate::{
    sync::worker::MessageHandler,
    util::HistoryRing,
    workers::msgs::{GameTelemetry, RendererTelemetry},
};

pub const HISTORY_CAPACITY: usize = 128;

pub struct UIState {
    pub frame_time: f32,
    pub frame_time_hist: HistoryRing<f32>,
    pub tick_time: f32,
    pub tick_time_hist: HistoryRing<f32>,
}

impl MessageHandler<GameTelemetry> for UIState {
    fn handle(&mut self, msg: GameTelemetry) {
        self.tick_time = msg.tick_time;
        self.tick_time_hist.write(msg.tick_time * 1000.0);
    }
}

impl MessageHandler<RendererTelemetry> for UIState {
    fn handle(&mut self, msg: RendererTelemetry) {
        self.frame_time = msg.frame_time;
        self.frame_time_hist.write(msg.frame_time * 1000.0);
    }
}

impl Default for UIState {
    fn default() -> Self {
        Self {
            frame_time: 0.0,
            frame_time_hist: HistoryRing::new(HISTORY_CAPACITY, 0.0),
            tick_time: 0.0,
            tick_time_hist: HistoryRing::new(HISTORY_CAPACITY, 0.0),
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

    Window::new("Debug").show(
        ctx,
        |ui| debug_window(state, resp, ui),
    );
}

fn debug_window(state: &UIState, resp: &mut UIResponse, ui: &mut Ui) {
    let tps = 1.0 / state.tick_time;
    let fps = 1.0 / state.frame_time;

    ui.label(format!("tick {:07.4}ms {tps:.0}/s", state.tick_time * 1000.0));
    sparkline(ui, "tick_time", &state.tick_time_hist);

    ui.label(format!("frame {:07.4}ms {fps:.0}/s", state.frame_time * 1000.0));
    sparkline(ui, "frame_time", &state.frame_time_hist);

    if ui.button("Quit").clicked() {
        resp.should_exit = true;
    }
}

fn sparkline(ui: &mut Ui, id: &str, hist: &HistoryRing<f32>) {
    let points: PlotPoints = hist.iter_head(f32::NAN)
        .enumerate()
        .map(|(i, v)| [i as f64, v.into()])
        .collect();

    let data = Line::new(id, points)
        .color(Color32::from_rgb(100, 200, 100))
        .width(1.5);

    let sweep = VLine::new(id, hist.head as f64)
        .color(Color32::from_rgba_unmultiplied(255, 0, 0, 200))
        .width(2.0);

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
                [HISTORY_CAPACITY as f64, 20.0]
            ));
            plot.line(data);
            plot.vline(sweep);
        });
}
