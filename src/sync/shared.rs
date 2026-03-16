use std::{cell::UnsafeCell, sync::atomic::{AtomicU64, Ordering}};

// single-writer seqlock
pub struct Shared<T> {
    seq: AtomicU64, // odd = write in progress, even = stable
    val: UnsafeCell<T>,
}

impl<T: Clone> Shared<T> {
    // TODO: this is currently not safe lmao. need to statically enforce that there is only ever one writer
    pub fn write(&self, val: T) {
        self.seq.fetch_add(1, Ordering::Release);
        unsafe { *self.val.get() = val; }
        self.seq.fetch_add(1, Ordering::Release);
    }

    pub fn read(&self) -> T {
        loop {
            let g1 = self.seq.load(Ordering::Acquire);
            if g1 % 2 == 1 {
                continue;
            }

            let state = unsafe { (*self.val.get()).clone() };

            let g2 = self.seq.load(Ordering::Acquire);
            if g1 == g2 {
                return state;
            }
        }
    }
}
