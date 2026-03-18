use std::{
    any::type_name,
    cell::Cell,
    sync::{Arc, Condvar, Mutex, atomic::{AtomicU8, Ordering}},
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};

use crate::sync::chan::{Rx, RxBlocking};

pub trait MessageHandler<T> {
    fn handle(&mut self, msg: T);
}

pub trait WorkerRx<State>: Send + 'static {
    type Msg: Send + Sync + 'static;
    type PrimaryRx: Rx<Self::Msg>;
    type PrimaryTx: Send + 'static;
    type TxBundle: Send + 'static; // macro-generated TxBundle struct without primary

    fn new() -> (Self::PrimaryTx, Self::TxBundle, Self);

    // return the primary channel
    fn primary(&mut self) -> &mut Self::PrimaryRx;

    fn drain_primary(&mut self, state: &mut State)
    where State: MessageHandler<Self::Msg>;

    fn drain_aux(&mut self, state: &mut State);
}

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
        drop(self.suspended_ack.wait_while(flag, |f| !*f).unwrap());
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
    type Init: Send + 'static;

    // internal state created on resume and dropped on suspend.
    // Worker owns and maintains Option<State>, the implementor
    // only ever touches the raw State
    type State: Send + 'static + MessageHandler<<Self::Channels as WorkerRx<Self::State>>::Msg>;

    type Channels: WorkerRx<Self::State>;

    fn on_resume(&self, init: Self::Init) -> anyhow::Result<Self::State>;

    // do something with the state, default is drop
    fn on_suspend(_this: Self::State) {}

    fn on_tick(
        this: &mut Self::State,
        since_wake: Duration,
        since_tick: Duration,
        tick: usize,
    ) -> anyhow::Result<()>;

    fn run(self, ctrl: Arc<WorkerControl<Self::Init>>, rx: Self::Channels) {
        self.run_free(ctrl, rx);
    }
}

// {{{ sealed helper trait for implementing Workers
mod seal {
    pub trait Sealed {}
    impl<T: super::Worker> Sealed for T {}
}

pub trait WorkerExt: Worker + seal::Sealed {
    /// poll as quickly as possible
    fn run_free(
        self,
        ctrl: Arc<WorkerControl<Self::Init>>,
        rx: Self::Channels,
    );

    /// poll with a fixed tickrate and perform subtick event handling.
    /// only works with a blocking primary channel
    fn run_tickrate(
        self,
        ctrl: Arc<WorkerControl<Self::Init>>,
        rx: Self::Channels,
        tickrate_hz: f64,
    ) where
        <Self::Channels as WorkerRx<Self::State>>::PrimaryRx:
            RxBlocking<<Self::Channels as WorkerRx<Self::State>>::Msg>;

    fn main_loop(
        self,
        ctrl: Arc<WorkerControl<Self::Init>>,
        rx: Self::Channels,
        pre_tick: impl FnMut(&mut Self::Channels, &mut Self::State),
        post_tick: impl FnMut(&mut Self::State),
    );

    fn spawn() -> WorkerHandle<Self>;
}

impl<T: Worker> WorkerExt for T {
    fn run_free(
        self,
        ctrl: Arc<WorkerControl<Self::Init>>,
        rx: Self::Channels,
    ) {
        self.main_loop(ctrl, rx,
            |rx, this| rx.drain_primary(this),
            |_| {},
        );
    }

    fn run_tickrate(
        self,
        ctrl: Arc<WorkerControl<Self::Init>>,
        mut rx: Self::Channels,
        tickrate_hz: f64,
    ) where
        <Self::Channels as WorkerRx<Self::State>>::PrimaryRx:
            RxBlocking<<Self::Channels as WorkerRx<Self::State>>::Msg>
    {
        let tick_dur = Duration::from_secs_f64(1.0 / tickrate_hz);
        let next = Cell::new(Instant::now() + tick_dur);

        // register this thread so the producer can unpark it
        rx.primary().use_current_thread();

        self.main_loop(ctrl, rx,
            |rx, this| {
                rx.primary().drain_deadline(next.get(), |msg| this.handle(msg));
            },
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

    // pre_tick should drain the primary queue, but not the auxiliary queues.
    // note that if you implement your own run() without using main_loop,
    // neither the primary or auxiliary event queues will be automatically
    // pumped! you must use methods on WorkerChannels to do it yourself
    fn main_loop(
        self,
        ctrl: Arc<WorkerControl<Self::Init>>,
        mut rx: Self::Channels,
        mut pre_tick: impl FnMut(&mut Self::Channels, &mut Self::State),
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

            pre_tick(&mut rx, o);
            rx.drain_aux(o);

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

        let (tx, aux_tx, rx) = Self::Channels::new();
        let thread = thread::spawn(move || {
            Self::default().run(ctrl_moved, rx);
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
    pub tx: <W::Channels as WorkerRx<W::State>>::PrimaryTx,
    pub aux_tx: <W::Channels as WorkerRx<W::State>>::TxBundle,
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

// {{{ declarative configuration of all channels a worker has
// this is very horrible and I don't think we should touch it. If new features need to
// be added we should probably make this a proc macro instead of using macro_rules

/// Generates channel bundles for Workers (or any other MessageHandler).
///
/// This macro aggregates a primary command channel and multiple auxiliary data
/// channels into unified tx/rx bundle structs. It routes incoming auxiliary messages
/// to their appropriate handlers using the `MessageHandler<T>::handle` method.
///
/// If a primary channel is provided, the macro generates a blanket impl of
/// WorkerRx<S> for all S: MessageHandler<PrimaryMsgType>.
///
/// Auxiliary fields are internally prefixed as `[source]_[field]` (e.g., `game_telemetry`)
/// to prevent namespace collisions if multiple sources send data to fields with the same name.
///
/// ```
/// channel_routing_for!(
///     PrefixName {
///         // optionally define the primary channel (required for Worker usage)
///         // "blocking" can be omitted to have a non-blocking primary channel
///         primary: [PrimaryMsgType; Capacity] blocking,
///
///         // auxiliary channels grouped by source
///         SourceName => {
///             field_name: [AuxMsgType; Capacity],
///         }
///     }
/// );
/// ```
///
/// # Generated Structs
/// For a given `$prefix`:
/// - `[$prefix]RxBundle`: Owns primary and auxiliary consumers.
/// - `[$prefix]TxBundle`: Owns all auxiliary producers, wrapped in `Option`
///                        so they can be taken and sent to other threads.
/// - `[$source]To[$prefix]`: Owns producers a specific source needs.
#[macro_export]
macro_rules! channel_routing_for {
    // with primary channel (for Worker impls)
    (
        $prefix:ident {
            primary : [ $pmsg:ty ; $pcap:expr ] $($blocking:ident)?
            $(, $source:ident => {
                $( $field:ident : [ $amsg:ty ; $acap:expr ] ),* $(,)?
            })* $(,)?
        }
    ) => {
        $crate::channel_routing_for!(@aux_tx $prefix $($source { $($field: $amsg),* })*);
        paste::paste! {
            // {{{ RxBundle struct owns all consumers
            pub struct [<$prefix RxBundle>] {
                primary: $crate::channel_routing_for!(@rx_type $($blocking)? $pmsg),
                $($([<$source:snake _ $field>]: rtrb::Consumer<$amsg>,)*)*
            }
            // }}}

            // {{{ Rx<PrimaryMsg> for all primary queue impls
            impl $crate::sync::chan::Rx<$pmsg> for [<$prefix RxBundle>] {
                #[inline(always)]
                fn pop(&mut self) -> Result<$pmsg, rtrb::PopError> {
                    $crate::sync::chan::Rx::pop(&mut self.primary)
                }
            }
            // }}}

            // {{{ RxBlocking for blocking primary queue
            $crate::channel_routing_for!(
                @impl_rx_blocking $($blocking)? $prefix $pmsg
            );
            // }}}

            // {{{ WorkerRx
            impl<State> $crate::sync::worker::WorkerRx<State>
            for [<$prefix RxBundle>]
            where
                State: $crate::sync::worker::MessageHandler<$pmsg>
                    $($( + $crate::sync::worker::MessageHandler<$amsg> )*)*
            {
                type Msg = $pmsg;
                type PrimaryRx = $crate::channel_routing_for!(
                    @rx_type $($blocking)? $pmsg
                );
                type PrimaryTx = $crate::channel_routing_for!(
                    @tx_type $($blocking)? $pmsg
                );
                type TxBundle = [<$prefix TxBundle>];

                fn new() -> (Self::PrimaryTx, Self::TxBundle, Self) {
                    let (primary_tx, primary_rx) = $crate::channel_routing_for!(
                        @construct $($blocking)? $pcap
                    );
                    $($(
                        let ( [<$source:snake _ $field _tx>], [<$source:snake _ $field _rx>] ) = rtrb::RingBuffer::new($acap);
                    )*)*
                    (
                        primary_tx,
                        [<$prefix TxBundle>] {
                            $($([<$source:snake _ $field>]: Some([<$source:snake _ $field _tx>]),)*)*
                        },
                        Self {
                            primary: primary_rx,
                            $($([<$source:snake _ $field>]: [<$source:snake _ $field _rx>],)*)*
                        },
                    )
                }

                fn primary(&mut self) -> &mut Self::PrimaryRx {
                    &mut self.primary
                }

                #[allow(unused_imports)]
                fn drain_primary(&mut self, state: &mut State) {
                    use $crate::sync::chan::Rx;
                    while let Ok(msg) = self.primary.pop() {
                        $crate::sync::worker::MessageHandler::handle(state, msg);
                    }
                }

                #[allow(unused_variables)]
                fn drain_aux(&mut self, state: &mut State) {
                    $crate::channel_routing_for!(@drain_aux self state $($source { $($field),* })*);
                }
            }
            // }}}
        }
    };

    // without primary channel (non-Worker handler like App)
    (
        $prefix:ident {
            $( $source:ident => {
                $( $field:ident : [ $amsg:ty ; $acap:expr ] ),* $(,)?
            }),+ $(,)?
        }
    ) => {
        $crate::channel_routing_for!(@aux_tx $prefix $($source { $($field: $amsg),* })*);
        paste::paste! {
            // {{{ RxBundle struct owns all consumers
            pub struct [<$prefix RxBundle>] {
                $($([<$source:snake _ $field>]: rtrb::Consumer<$amsg>,)*)*
            }

            impl [<$prefix RxBundle>] {
                pub fn drain_all<D>(&mut self, dest: &mut D)
                where D: $($($crate::sync::worker::MessageHandler<$amsg> +)*)* Sized {
                    $crate::channel_routing_for!(@drain_aux self dest $($source { $($field),* })*);
                }
            }
            // }}}

            // {{{ TxBundle struct handles construction
            impl [<$prefix TxBundle>] {
                pub fn new_pair() -> ([<$prefix TxBundle>], [<$prefix RxBundle>]) {
                    $($(
                        let ( [<$source:snake _ $field _tx>], [<$source:snake _ $field _rx>] ) = rtrb::RingBuffer::new($acap);
                    )*)*
                    (
                        [<$prefix TxBundle>] {
                            $($([<$source:snake _ $field>]: Some([<$source:snake _ $field _tx>]),)*)*
                        },
                        [<$prefix RxBundle>] {
                            $($([<$source:snake _ $field>]: [<$source:snake _ $field _rx>],)*)*
                        },
                    )
                }
            }
            // }}}
        }
    };

    (@aux_tx $prefix:ident $($source:ident { $($field:ident: $amsg:ty),* })*) => {
        paste::paste! {
            // {{{ TxBundle struct with take_$source()
            pub struct [<$prefix TxBundle>] {
                $($(pub [<$source:snake _ $field>]: Option<rtrb::Producer<$amsg>>,)*)*
            }

            $(
                pub struct [<$source To $prefix>] {
                    $(pub $field: rtrb::Producer<$amsg>,)*
                }
            )*

            impl [<$prefix TxBundle>] {
                $(
                    pub fn [<take_ $source:snake>](
                        &mut self,
                    ) -> Option<[<$source To $prefix>]> {
                        Some([<$source To $prefix>] {
                            $($field: self.[<$source:snake _ $field>].take()?,)*
                        })
                    }
                )*
            }
            // }}}
        }
    };

    // drain aux consumers into dest
    (@drain_aux $self:ident $dest:ident $($source:ident { $($field:ident),* })*) => {
        paste::paste! {
            $($(
                while let Ok(msg) = $self.[<$source:snake _ $field>].pop() {
                    $crate::sync::worker::MessageHandler::handle($dest, msg);
                }
            )*)*
        }
    };

    // {{{ impl RxBlocking
    (@impl_rx_blocking blocking $prefix:ident $pmsg:ty) => {
        paste::paste! {
            impl $crate::sync::chan::RxBlocking<$pmsg> for [<$prefix RxBundle>] {
                fn use_current_thread(&self) {
                    $crate::sync::chan::RxBlocking::use_current_thread(
                        &self.primary
                    );
                }
                fn recv(&mut self) -> $pmsg {
                    $crate::sync::chan::RxBlocking::recv(&mut self.primary)
                }
                fn recv_deadline(
                    &mut self,
                    deadline: std::time::Instant,
                ) -> Option<$pmsg> {
                    $crate::sync::chan::RxBlocking::recv_deadline(
                        &mut self.primary,
                        deadline,
                    )
                }
            }
        }
    };

    // no keyword, don't do impl
    (@impl_rx_blocking $prefix:ident $pmsg:ty) => {};
    // }}}

    (@tx_type blocking $msg:ty) => { $crate::sync::chan::ProducerBlocking<$msg> };
    (@tx_type          $msg:ty) => { rtrb::Producer<$msg> };
    (@rx_type blocking $msg:ty) => { $crate::sync::chan::ConsumerBlocking<$msg> };
    (@rx_type          $msg:ty) => { rtrb::Consumer<$msg> };

    (@construct blocking $cap:expr) => { $crate::sync::chan::bounded_deadline($cap) };
    (@construct          $cap:expr) => { rtrb::RingBuffer::new($cap) };
}

pub(crate) use channel_routing_for;
// }}}
