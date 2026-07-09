#![allow(dead_code)]
mod gpu;
mod sync;
mod ui;
mod workers;
mod world;

mod app;
mod util;

use winit::event_loop::EventLoop;

use crate::app::App;

fn main() -> anyhow::Result<()> {
    let evt_loop = EventLoop::new()?;
    let mut app = App::new();
    evt_loop.run_app(&mut app)?;
    Ok(())
}
