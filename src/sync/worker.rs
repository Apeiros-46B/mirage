use std::{
    any::type_name, cell::Cell, sync::{Arc, atomic::{AtomicBool, Ordering}}, thread::{self, JoinHandle}, time::{Duration, Instant}
};

use crate::sync::chan::{
    ProducerOneshot, Rx, RxDeadline, Tx, oneshot
};

// {{{ declarative auxiliary channel setup for workers
pub trait AuxTx<W>: Sized + Send + 'static {
    type Rx: AuxRx<W>;
    /// construct all channel pairs
    fn new() -> (Self, Self::Rx);
}

pub trait AuxRx<W>: Send + 'static {
    /// drains all channels without blocking, routing messages to worker methods
    fn drain_all(&mut self, worker: &mut W);
}

// workers with no auxiliary channels
impl<W> AuxTx<W> for () {
    type Rx = ();
    fn new() -> ((), ()) { ((), ()) }
}

impl<W> AuxRx<W> for () {
    fn drain_all(&mut self, _worker: &mut W) {}
}

#[macro_export]
macro_rules! inbound_aux_channels {
    (
        $worker:ident, {
            $(
                $source:ident => {
                    $( $field:ident : [ $msg:ty ; $size:expr ] => $handler:ident ),* $(,)?
                }
            ),* $(,)?
        }
    ) => {
        paste::paste! {
            pub struct [<$worker AuxTx>] {
                $(
                    $(pub $field: Option<rtrb::Producer<$msg>>,)*
                )*
            }

            pub struct [<$worker AuxRx>] {
                $(
                    $(pub $field: rtrb::Consumer<$msg>,)*
                )*
            }

            $(
                pub struct [<$source To $worker>] {
                    $(pub $field: rtrb::Producer<$msg>,)*
                }
            )*

            impl [<$worker AuxTx>] {
                $(
                    pub fn [<take_ $source:snake>](&mut self) -> Option<[<$source To $worker>]> {
                        Some([<$source To $worker>] {
                            $(
                                $field: self.$field.take()?,
                            )*
                        })
                    }
                )*
            }

            impl $crate::sync::worker::AuxTx<$worker> for [<$worker AuxTx>] {
                type Rx = [<$worker AuxRx>];

                fn new() -> (Self, Self::Rx) {
                    $(
                        $(
                            let ([<$field _tx>], [<$field _rx>]) = rtrb::RingBuffer::new($size);
                        )*
                    )*
                    (
                        Self { $($($field: Some([<$field _tx>]),)*)* },
                        Self::Rx { $($($field: [<$field _rx>],)*)* },
                    )
                }
            }

            impl $crate::sync::worker::AuxRx<$worker> for [<$worker AuxRx>] {
                fn drain_all(&mut self, worker: &mut $worker) {
                    $(
                        $(
                            while let Ok(msg) = self.$field.pop() {
                                worker.$handler(msg);
                            }
                        )*
                    )*
                }
            }
        }
    };
}

pub(crate) use inbound_aux_channels;
// }}}

pub trait Worker: Sized {
    type Init: Send + 'static;
    type Msg: Send + Sync + 'static;

    type Tx: Tx<Self::Msg>;
    type Rx: Rx<Self::Msg>;

    type AuxTx: AuxTx<Self, Rx = Self::AuxRx>;
    type AuxRx: AuxRx<Self>;

    fn channel() -> (Self::Tx, Self::Rx);

    fn new(init: Self::Init) -> anyhow::Result<Self>;

    fn tick(&mut self, now: Duration, dt: Duration, tick: usize) -> anyhow::Result<()>;

    fn handle(&mut self, msg: Self::Msg);

    fn run(
        self,
        rx: Self::Rx,
        aux_rx: Self::AuxRx,
        should_exit: Arc<AtomicBool>,
    ) {
        self.run_free(rx, aux_rx, should_exit);
    }
}

// {{{ helper methods for implementing Workers
mod seal {
    pub trait Sealed {}
    impl<T: super::Worker> Sealed for T {}
}

pub trait WorkerExt: Worker + seal::Sealed {
    fn aux_channels() -> (Self::AuxTx, Self::AuxRx);

    /// poll as quickly as possible
    fn run_free(
        self,
        rx: Self::Rx,
        aux_rx: Self::AuxRx,
        should_exit: Arc<AtomicBool>,
    );

    /// poll with a fixed tickrate and perform subtick event handling
    fn run_tickrate(
        self,
        rx: Self::Rx,
        aux_rx: Self::AuxRx,
        should_exit: Arc<AtomicBool>,
        tickrate_hz: f64,
    ) where Self::Rx: RxDeadline<Self::Msg>;

    fn main_loop(
        &mut self,
        should_exit: Arc<AtomicBool>,
        aux_rx: Self::AuxRx,
        pre_tick: impl FnMut(&mut Self),
        post_tick: impl FnMut(&mut Self),
    );

    fn spawn() -> WorkerHandle<Self>;
}

impl<T: Worker> WorkerExt for T {
    fn aux_channels() -> (Self::AuxTx, Self::AuxRx)  {
        <Self::AuxTx as AuxTx<Self>>::new()
    }

    fn run_free(
        mut self,
        mut rx: Self::Rx,
        aux_rx: Self::AuxRx,
        should_exit: Arc<AtomicBool>,
    ) {
        self.main_loop(should_exit, aux_rx,
            |this| rx.drain(|msg| this.handle(msg)),
            |_| {},
        );
    }

    fn run_tickrate(
        mut self,
        mut rx: Self::Rx,
        aux_rx: Self::AuxRx,
        should_exit: Arc<AtomicBool>,
        tickrate_hz: f64,
    ) where Self::Rx: RxDeadline<Self::Msg> {
        let tick_dur = Duration::from_secs_f64(1.0 / tickrate_hz);
        let next = Cell::new(Instant::now() + tick_dur);

        // register this thread so the producer can unpark it
        rx.use_current_thread();

        self.main_loop(should_exit, aux_rx,
            |this| rx.drain_deadline(next.get(), |msg| this.handle(msg)),
            |_| {
                // advance the deadline by integer multiples of tick time.
                // if tick() processing lags and misses ticks, skip them
                let now = Instant::now();
                while next.get() <= now {
                    next.update(|val| val + tick_dur);
                }
            },
        );
    }

    fn main_loop(
        &mut self,
        should_exit: Arc<AtomicBool>,
        mut aux_rx: Self::AuxRx,
        mut pre_tick: impl FnMut(&mut Self),
        mut post_tick: impl FnMut(&mut Self),
    ) {
        let start = Instant::now();
        let mut prev = start;
        let mut tick = 0usize;

        loop {
            if should_exit.load(Ordering::Relaxed) {
                return;
            }

            pre_tick(self);

            // not sure if this should be before or after pre_tick
            aux_rx.drain_all(self);

            let now = Instant::now();
            if let Err(e) = self.tick(now - start, now - prev, tick) {
                eprintln!("[{}] tick {} error: {e}", type_name::<Self>(), tick);
            }
            prev = now;
            tick += 1;

            post_tick(self);
        }
    }

    fn spawn() -> WorkerHandle<Self> {
        let should_exit = Arc::new(AtomicBool::new(false));
        let should_exit_moved = should_exit.clone();

        let (itx, irx) = oneshot();
        let (tx, rx) = Self::channel();
        let (aux_tx, aux_rx) = Self::aux_channels();

        let thread = thread::spawn(move || {
            // blocks until we receive initialization data
            let Some(init) = irx.recv() else {
                return
            };
            Self::new(init)
                .expect(&format!("failed to init {}", type_name::<Self>()))
                .run(rx, aux_rx, should_exit_moved);
        });

        WorkerHandle {
            thread: Some(thread),
            should_exit,
            itx: Some(itx),
            tx,
            aux_tx,
        }
    }
}
// }}}

// {{{ handle for communication with a spawned worker
pub struct WorkerHandle<W: Worker> {
    thread: Option<JoinHandle<()>>,
    pub should_exit: Arc<AtomicBool>,

    itx: Option<ProducerOneshot<W::Init>>,
    pub tx: W::Tx,
    pub aux_tx: W::AuxTx,
}

impl<W: Worker> WorkerHandle<W> {
    pub fn init(&mut self, init: W::Init) {
        if let Some(itx) = self.itx.take() {
            itx.send(init);
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
// }}}
