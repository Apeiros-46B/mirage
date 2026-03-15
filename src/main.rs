mod app;
mod game;
mod gpu;
mod render;
mod error;
mod sync;
mod util;

use winit::event_loop::EventLoop;

use crate::app::App;

fn main() -> anyhow::Result<()> {
    let evt_loop = EventLoop::new()?;
    let mut app = App::new();
    evt_loop.run_app(&mut app)?;
    Ok(())
}
