use std::{cell::UnsafeCell, sync::{Arc, atomic::{AtomicU64, Ordering}}};

// single-writer seqlock for Copy values
struct Shared<T> {
    seq: AtomicU64, // odd = write in progress, even = stable
    val: UnsafeCell<T>,
}

// Send+Sync are safe because SharedWriter enforces single-writer
unsafe impl<T: Send> Send for Shared<T> {}
unsafe impl<T: Sync> Sync for Shared<T> {}

// not Clone, there can only exist one writer per Shared
pub struct SharedWriter<T>(Arc<Shared<T>>);

#[derive(Clone)]
pub struct SharedReader<T>(Arc<Shared<T>>);

pub fn shared<T>(val: T) -> (SharedWriter<T>, SharedReader<T>) {
    let inner = Arc::new(Shared {
        seq: AtomicU64::new(0),
        val: UnsafeCell::new(val),
    });
    (SharedWriter(inner.clone()), SharedReader(inner))
}

impl<T> SharedWriter<T> {
    pub fn write(&self, val: T) {
        self.0.seq.fetch_add(1, Ordering::Release);
        unsafe { *self.0.val.get() = val; }
        self.0.seq.fetch_add(1, Ordering::Release);
    }

    pub fn reader(&self) -> SharedReader<T> {
        SharedReader(self.0.clone())
    }
}

impl<T: Copy> SharedReader<T> {
    pub fn read(&self) -> T {
        loop {
            let seq1 = self.0.seq.load(Ordering::Acquire);
            if seq1 % 2 == 1 { continue; } // busy wait

            let val = unsafe { self.0.val.get().read() };

            let seq2 = self.0.seq.load(Ordering::Acquire);
            if seq1 == seq2 { return val; }
        }
    }
}
