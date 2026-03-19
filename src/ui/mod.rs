use egui::{Context, Ui, Window};
use winit::event_loop::ActiveEventLoop;

#[derive(Default)]
pub struct UIState {
    pub frame_time: f32,
    pub tick_time: f32,
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
    Window::new("Debug").show(
        ctx,
        |ui| debug_window(state, resp, ui),
    );
}

fn debug_window(state: &UIState, resp: &mut UIResponse, ui: &mut Ui) {
    let tps = 1.0 / state.tick_time;
    let fps = 1.0 / state.frame_time;

    ui.label(format!("tick {:07.4}ms {tps:.0}/s", state.tick_time * 1000.0));
    ui.label(format!("frame {:07.4}ms {fps:.0}/s", state.frame_time * 1000.0));
    if ui.button("Quit").clicked() {
        resp.should_exit = true;
    }
}
