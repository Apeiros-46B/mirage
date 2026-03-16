use std::{
    sync::{Arc, atomic::{AtomicBool, Ordering}},
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};

use crate::sync::chan::{DeadlineRx, OneshotProducer, Rx, Tx, oneshot};

pub trait Worker: Sized {
    const NAME: &'static str;

    type Init: Send + 'static;
    type Msg: Send + Sync + 'static;
    type Tx: Tx<Self::Msg>;
    type Rx: Rx<Self::Msg>;

    fn channel() -> (Self::Tx, Self::Rx);

    fn new(init: Self::Init) -> anyhow::Result<Self>;

    fn tick(&mut self, now: Duration, dt: Duration, tick: usize) -> anyhow::Result<()>;

    fn handle(&mut self, msg: Self::Msg);

    // poll as fast as possible with no timing guarantees
    fn run_free(mut self, mut rx: Self::Rx, should_exit: Arc<AtomicBool>) {
        let mut prev = Instant::now();
        let mut tick = 0usize;

        self.main_loop(should_exit, &mut prev, &mut tick,
            |this| rx.drain(|msg| this.handle(msg)),
            |_| {},
        );
    }

    // poll with a fixed tickrate and wake up on receiving
    fn run_tickrate(
        mut self,
        mut rx: Self::Rx,
        should_exit: Arc<AtomicBool>,
        tickrate_hz: f64,
    ) where Self::Rx: DeadlineRx<Self::Msg> {
        let tick_dur = Duration::from_secs_f64(1.0 / tickrate_hz);

        let mut next = Instant::now() + tick_dur;
        let mut prev = Instant::now();
        let mut tick = 0usize;

        let deadline = next;

        // register this thread so the producer can unpark it
        rx.use_current_thread();

        self.main_loop(should_exit, &mut prev, &mut tick,
            |this| rx.drain_deadline(deadline, |msg| this.handle(msg)),
            |_| {
                // advance the deadline by integer multiples of tick time.
                // if the tick processing lags, missing later ticks, skip them
                let now = Instant::now();
                while next <= now {
                    next += tick_dur;
                }
            },
        );
    }

    fn main_loop(
        &mut self,
        should_exit: Arc<AtomicBool>,
        prev: &mut Instant,
        tick: &mut usize,
        mut pre_tick: impl FnMut(&mut Self),
        mut post_tick: impl FnMut(&mut Self),
    ) {
        let start = Instant::now();

        loop {
            if should_exit.load(Ordering::Relaxed) {
                return;
            }

            pre_tick(self);

            let now = Instant::now();
            if let Err(e) = self.tick(now - start, now - *prev, *tick) {
                eprintln!("[{}] tick {} error: {e}", Self::NAME, tick);
            }
            *prev = now;
            *tick += 1;

            post_tick(self);
        }
    }

    fn run(self, rx: Self::Rx, should_exit: Arc<AtomicBool>) {
        self.run_free(rx, should_exit);
    }

    fn spawn() -> WorkerHandle<Self> {
        let should_exit = Arc::new(AtomicBool::new(false));
        let should_exit_moved = should_exit.clone();

        let (itx, irx) = oneshot();
        let (tx, rx) = Self::channel();
        let thread = thread::spawn(move || {
            // blocks until we receive initialization data
            let Some(init) = irx.recv() else {
                return
            };
            Self::new(init)
                .expect(&format!("failed to init {}", Self::NAME))
                .run(rx, should_exit_moved);
        });

        WorkerHandle {
            thread: Some(thread),
            should_exit,
            itx: Some(itx),
            tx,
        }
    }
}

pub struct WorkerHandle<W: Worker> {
    thread: Option<JoinHandle<()>>,
    pub should_exit: Arc<AtomicBool>,

    itx: Option<OneshotProducer<W::Init>>,
    tx: W::Tx,
}

impl<W: Worker> WorkerHandle<W> {
    pub fn init(&mut self, init: W::Init) {
        if let Some(itx) = self.itx.take() {
            itx.send(init);
        }
    }

    pub fn send(&mut self, msg: W::Msg) {
        if let Err(err) = self.tx.push(msg) {
            eprintln!("failed to send to {}: {}", W::NAME, err);
        }
    }

    /// signals the worker to stop and blocks until the thread finishes its current tick.
    /// idempotent
    pub fn exit(&mut self) {
        self.should_exit.store(true, Ordering::Relaxed);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

impl<W: Worker> Drop for WorkerHandle<W> {
    fn drop(&mut self) {
        self.exit();
    }
}
