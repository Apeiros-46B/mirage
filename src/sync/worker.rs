use std::{
    any::type_name,
    cell::Cell,
    sync::{Arc, Condvar, Mutex, atomic::{AtomicU8, Ordering}},
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};

use crate::sync::chan::{
    Rx, RxBlocking, RxBundle, Tx, TxBundle
};

// {{{ worker control
pub struct WorkerStatus;

impl WorkerStatus {
    pub const DEAD: u8 = 0;
    pub const ALIVE: u8 = 1;
    pub const SUSPENDED: u8 = 2;
}

pub struct WorkerControl<I> {
    status: AtomicU8,
    init_payload: Mutex<Option<I>>,
    suspended_ack: Condvar,
    suspended_flag: Mutex<bool>,
    // TODO: fix potential edge case where WorkerHandle's unpark() call unparks the
    // drain_deadline instead of the suspended park, causing the worker to deadlock
}

impl<I> WorkerControl<I> {
    // called by control thread
    fn resume(&self, init: I) {
        // reset ack flag before transitioning to ALIVE
        *self.suspended_flag.lock().unwrap() = false;

        // hold lock across ops so worker can't get ALIVE with no payload
        let mut payload = self.init_payload.lock().unwrap();
        *payload = Some(init);
        self.status.store(WorkerStatus::ALIVE, Ordering::Release);
    }

    // called by control thread
    fn wait_until_suspend_done(&self) {
        let flag = self.suspended_flag.lock().unwrap();
        self.suspended_ack.wait_while(flag, |f| !*f).unwrap();
    }

    // called by worker thread
    fn notify_suspend(&self) {
        let mut flag = self.suspended_flag.lock().unwrap();
        *flag = true;
        self.suspended_ack.notify_all();
    }
}
// }}}

pub trait Worker: Sized + Default {
    // internal state created on resume and dropped on suspend.
    // Worker owns and maintains Option<State>, the implementor
    // only ever touches the raw State
    type State: Send + 'static;

    type Init: Send + 'static;
    type Msg: Send + Sync + 'static;

    type Tx: Tx<Self::Msg>;
    type Rx: Rx<Self::Msg>;

    type AuxTx: TxBundle<Self::State, Rx = Self::AuxRx>;
    type AuxRx: RxBundle<Self::State>;

    fn channel() -> (Self::Tx, Self::Rx);

    fn on_resume(&self, init: Self::Init) -> anyhow::Result<Self::State>;

    // do something with the state, default is drop
    fn on_suspend(_this: Self::State) {}

    fn on_tick(
        this: &mut Self::State,
        since_wake: Duration,
        since_tick: Duration,
        tick: usize,
    ) -> anyhow::Result<()>;

    fn on_msg(this: &mut Self::State, msg: Self::Msg);

    fn run(
        self,
        ctrl: Arc<WorkerControl<Self::Init>>,
        rx: Self::Rx,
        aux_rx: Self::AuxRx,
    ) {
        self.run_free(ctrl, rx, aux_rx);
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
        ctrl: Arc<WorkerControl<Self::Init>>,
        rx: Self::Rx,
        aux_rx: Self::AuxRx,
    );

    /// poll with a fixed tickrate and perform subtick event handling
    fn run_tickrate(
        self,
        ctrl: Arc<WorkerControl<Self::Init>>,
        rx: Self::Rx,
        aux_rx: Self::AuxRx,
        tickrate_hz: f64,
    ) where Self::Rx: RxBlocking<Self::Msg>;

    fn main_loop(
        self,
        ctrl: Arc<WorkerControl<Self::Init>>,
        aux_rx: Self::AuxRx,
        pre_tick: impl FnMut(&mut Self::State),
        post_tick: impl FnMut(&mut Self::State),
    );

    fn spawn() -> WorkerHandle<Self>;
}

impl<T: Worker> WorkerExt for T {
    fn aux_channels() -> (Self::AuxTx, Self::AuxRx)  {
        <Self::AuxTx as TxBundle<Self::State>>::new()
    }

    fn run_free(
        self,
        ctrl: Arc<WorkerControl<Self::Init>>,
        mut rx: Self::Rx,
        aux_rx: Self::AuxRx,
    ) {
        self.main_loop(ctrl, aux_rx,
            |this| rx.drain(|msg| Self::on_msg(this, msg)),
            |_| {},
        );
    }

    fn run_tickrate(
        self,
        ctrl: Arc<WorkerControl<Self::Init>>,
        mut rx: Self::Rx,
        aux_rx: Self::AuxRx,
        tickrate_hz: f64,
    ) where Self::Rx: RxBlocking<Self::Msg> {
        let tick_dur = Duration::from_secs_f64(1.0 / tickrate_hz);
        let next = Cell::new(Instant::now() + tick_dur);

        // register this thread so the producer can unpark it
        rx.use_current_thread();

        self.main_loop(ctrl, aux_rx,
            |this| rx.drain_deadline(next.get(), |msg| Self::on_msg(this, msg)),
            |_| {
                // advance the deadline by integer multiples of tick time.
                // if tick() processing lags and misses ticks, skip them.
                // this also fast-forwards from the stale timestamp when
                // we are suspended and then later resumed in the future
                let now = Instant::now();
                while next.get() <= now {
                    next.update(|val| val + tick_dur);
                }
            },
        );
    }

    fn main_loop(
        self,
        ctrl: Arc<WorkerControl<Self::Init>>,
        mut aux_rx: Self::AuxRx,
        mut pre_tick: impl FnMut(&mut Self::State),
        mut post_tick: impl FnMut(&mut Self::State),
    ) {
        let mut this = None;
        let mut start = Instant::now();
        let mut prev = start;
        let mut tick = 0usize;

        loop {
            match ctrl.status.load(Ordering::Acquire) {
                status @ (WorkerStatus::DEAD | WorkerStatus::SUSPENDED) => {
                    if let Some(o) = this.take() {
                        Self::on_suspend(o);
                        ctrl.notify_suspend();
                    }
                    if status == WorkerStatus::DEAD {
                        return;
                    }
                }
                WorkerStatus::ALIVE if this.is_none() => {
                    let mut payload = ctrl.init_payload.lock().unwrap();
                    if let Some(init) = payload.take() {
                        match self.on_resume(init) {
                            Ok(o) => {
                                this = Some(o);
                                start = Instant::now();
                                prev = start;
                                tick = 0;
                            }
                            Err(e) => eprintln!(
                                "failed to wake {}: {e}",
                                type_name::<Self>(),
                            ),
                        }
                    }
                }
                _ => {}
            }

            let Some(o) = this.as_mut() else {
                thread::park();
                continue;
            };

            pre_tick(o);
            aux_rx.drain_all(o);

            let now = Instant::now();
            if let Err(e) = Self::on_tick(o, now - start, now - prev, tick) {
                eprintln!("{} tick {tick} error: {e}", type_name::<Self>());
            }
            prev = now;
            tick += 1;

            post_tick(o);
        }
    }

    fn spawn() -> WorkerHandle<Self> {
        let ctrl = Arc::new(WorkerControl {
            status: AtomicU8::new(WorkerStatus::SUSPENDED),
            init_payload: Mutex::new(None),
            suspended_ack: Condvar::new(),
            suspended_flag: Mutex::new(false),
        });
        let ctrl_moved = ctrl.clone();

        let (tx, rx) = Self::channel();
        let (aux_tx, aux_rx) = Self::aux_channels();

        let thread = thread::spawn(move || {
            Self::default().run(ctrl_moved, rx, aux_rx);
        });

        WorkerHandle {
            thread: Some(thread),
            ctrl,
            tx,
            aux_tx,
        }
    }
}
// }}}

// {{{ handle for communication with a spawned worker
pub struct WorkerHandle<W: Worker> {
    thread: Option<JoinHandle<()>>,
    ctrl: Arc<WorkerControl<W::Init>>,
    pub tx: W::Tx,
    pub aux_tx: W::AuxTx,
}

impl<W: Worker> WorkerHandle<W> {
    pub fn resume(&mut self, init: W::Init) {
        self.ctrl.resume(init);
        if let Some(t) = &self.thread {
            t.thread().unpark();
        }
    }

    /// this may deadlock if called before the worker has processed a preceding resume().
    /// suspend() should only be called on a worker that is known to be alive
    pub fn suspend(&mut self) {
        if self.ctrl.status.load(Ordering::Acquire) == WorkerStatus::SUSPENDED {
            return; // already suspended
        }
        self.ctrl.status.store(WorkerStatus::SUSPENDED, Ordering::Release);

        if let Some(t) = &self.thread {
            t.thread().unpark();
        }
        self.ctrl.wait_until_suspend_done();
    }

    /// signals the worker to stop and blocks until it finishes its current tick
    pub fn exit(&mut self) {
        self.ctrl.status.store(WorkerStatus::DEAD, Ordering::Release);

        if let Some(thread) = self.thread.take() {
            thread.thread().unpark();
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
