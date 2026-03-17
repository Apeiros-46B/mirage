// utility
mod gpu;
mod sync;
mod world;

// workers
mod audio;
mod game;
mod render;
mod stream;

mod app;
mod error;
mod util;

use winit::event_loop::EventLoop;

use crate::app::App;

fn main() -> anyhow::Result<()> {
    let evt_loop = EventLoop::new()?;
    let mut app = App::new();
    evt_loop.run_app(&mut app)?;
    Ok(())
}
