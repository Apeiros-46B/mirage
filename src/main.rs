// utility
mod debug;
mod gpu;
mod sync;
mod world;

mod workers;

mod app;
mod error;

use winit::event_loop::EventLoop;

use crate::app::App;

fn main() -> anyhow::Result<()> {
    let evt_loop = EventLoop::new()?;
    let mut app = App::new();
    evt_loop.run_app(&mut app)?;
    Ok(())
}
