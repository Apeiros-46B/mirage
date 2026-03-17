use std::{cell::UnsafeCell, sync::atomic::AtomicU32};

use rtrb::Producer;

pub struct Telemetry {
    // f32 stored bitwise as u32
    // for text labels
    pub game_mspt: AtomicU32,
    pub render_mspf: AtomicU32,

    // for sparklines
    game_mspt_tx: UnsafeCell<Option<Producer<f32>>>,
    render_mspf_tx: UnsafeCell<Option<Producer<f32>>>,
}

// safety: UnsafeCell fields are taken
unsafe impl Sync for Telemetry {}

impl Telemetry {
    pub fn take_game_mspt_tx(&self) -> Option<Producer<f32>> {
        unsafe { (*self.game_mspt_tx.get()).take() }
    }

    pub fn take_render_mspf_tx(&self) -> Option<Producer<f32>> {
        unsafe { (*self.render_mspf_tx.get()).take() }
    }
}
