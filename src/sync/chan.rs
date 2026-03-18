use std::{
    sync::{Arc, Condvar, Mutex, OnceLock, atomic::{AtomicBool, Ordering}},
    thread,
    time::Instant,
};

use rtrb::RingBuffer;

pub use rtrb::{Consumer, PopError, Producer, PushError};

pub trait Tx<T>: Send + 'static {
    fn push(&mut self, msg: T) -> Result<(), PushError<T>>;
}

pub trait Rx<T>: Send + 'static {
    fn pop(&mut self) -> Result<T, PopError>;

    fn drain(&mut self, mut f: impl FnMut(T)) {
        while let Ok(msg) = self.pop() {
            f(msg);
        }
    }
}

pub trait RxBlocking<T>: Rx<T> {
    // called before usage on the thread that will be parked
    fn use_current_thread(&self) {}

    fn recv(&mut self) -> T;

    fn recv_deadline(&mut self, deadline: Instant) -> Option<T>;

    fn drain_deadline<F>(&mut self, deadline: Instant, mut f: F)
    where F: FnMut(T)
    {
        while let Some(msg) = self.recv_deadline(deadline) {
            f(msg);

            // try to pop more without recv_deadline which may have expensive checks
            while let Ok(extra_msg) = self.pop() {
                f(extra_msg);

                // if processing batched messages times out, abort immediately
                if Instant::now() >= deadline {
                    return;
                }
            }

            // the queue is now empty but the deadline has not yet been reached
            // the `while let` will call pop_deadline again, blocking until timeout
        }
    }
}

// {{{ non-blocking wrapper for rtrb queues
impl<T: Send + 'static> Tx<T> for Producer<T> {
    #[inline(always)]
    fn push(&mut self, msg: T) -> Result<(), PushError<T>> {
        self.push(msg)
    }
}

impl<T: Send + 'static> Rx<T> for Consumer<T> {
    #[inline(always)]
    fn pop(&mut self) -> Result<T, PopError> {
        self.pop()
    }
}

pub fn bounded<T>(capacity: usize) -> (Producer<T>, Consumer<T>) {
    RingBuffer::new(capacity)
}
// }}}

// {{{ blocking wrapper for rtrb queues
pub struct ProducerBlocking<T> {
    tx: Producer<T>,
    sleeping: Arc<AtomicBool>,
    consumer: Arc<OnceLock<thread::Thread>>,
}

pub struct ConsumerBlocking<T> {
    rx: Consumer<T>,
    sleeping: Arc<AtomicBool>,
    thread_slot: Arc<OnceLock<thread::Thread>>,
}

pub fn bounded_deadline<T>(capacity: usize)
    -> (ProducerBlocking<T>, ConsumerBlocking<T>)
{
    let (tx, rx) = RingBuffer::new(capacity);
    let sleeping = Arc::new(AtomicBool::new(false));
    let slot = Arc::new(OnceLock::new());
    (
        ProducerBlocking { tx, sleeping: sleeping.clone(), consumer: slot.clone() },
        ConsumerBlocking { rx, sleeping, thread_slot: slot },
    )
}

impl<T: Send + 'static> Tx<T> for ProducerBlocking<T> {
    fn push(&mut self, msg: T) -> Result<(), PushError<T>> {
        self.tx.push(msg)?;
        if self.sleeping.load(Ordering::Acquire) {
            if let Some(t) = self.consumer.get() {
                t.unpark();
            }
        }
        Ok(())
    }
}

impl<T: Send + 'static> Rx<T> for ConsumerBlocking<T> {
    fn pop(&mut self) -> Result<T, PopError> {
        self.rx.pop()
    }
}

impl<T: Send + 'static> RxBlocking<T> for ConsumerBlocking<T> {
    fn use_current_thread(&self) {
        self.thread_slot.get_or_init(thread::current);
    }

    fn recv(&mut self) -> T {
        loop {
            if let Ok(msg) = self.rx.pop() {
                return msg;
            }

            self.sleeping.store(true, Ordering::Release);

            // re-check because sender might have pushed a message between our first
            // pop and the previous flag update
            if let Ok(msg) = self.rx.pop() {
                self.sleeping.store(false, Ordering::Release);
                return msg;
            }

            thread::park();

            self.sleeping.store(false, Ordering::Release);
        }
    }

    fn recv_deadline(&mut self, deadline: Instant) -> Option<T> {
        loop {
            if let Ok(msg) = self.rx.pop() {
                return Some(msg);
            }

            let now = Instant::now();
            if now >= deadline {
                return None;
            }

            self.sleeping.store(true, Ordering::Release);

            if let Ok(msg) = self.rx.pop() {
                self.sleeping.store(false, Ordering::Release);
                return Some(msg);
            }

            thread::park_timeout(deadline - now);

            self.sleeping.store(false, Ordering::Release);
        }
    }
}
// }}}

// {{{ send safely to a channel and log errors in debug mode
pub trait SafeSend<T> {
    fn send(&mut self, msg: T);
}

// need to duplicate once to prevent "downstream traits may implement" (E0119)
pub trait SafeSendOpt<T> {
    fn send(&mut self, msg: T);
}

impl<T, Q: Tx<T>> SafeSend<T> for Q {
    #[cfg(debug_assertions)]
    fn send(&mut self, msg: T) {
        if let Err(err) = self.push(msg) {
            eprintln!(
                "failed to send {}: {}",
                std::any::type_name::<T>(),
                err,
            );
        }
    }

    #[cfg(not(debug_assertions))]
    fn send(&mut self, msg: T) {
        let _ = self.push(msg);
    }
}

impl<T, Q: SafeSend<T>> SafeSendOpt<T> for Option<Q> {
    #[cfg(debug_assertions)]
    fn send(&mut self, msg: T) {
        if let Some(tx) = self {
            tx.send(msg);
        } else {
            eprintln!(
                "tried to send {} to taken queue",
                std::any::type_name::<T>(),
            );
        }
    }

    #[cfg(not(debug_assertions))]
    fn send(&mut self, msg: T) {
        if let Some(tx) = self {
            tx.send(msg);
        }
    }
}
// }}}

// {{{ declarative configuration of multi-lane message passing
pub trait MessageHandler<T> {
    fn handle(&mut self, msg: T);
}

pub trait TxBundle<D>: Sized + Send + 'static {
    type Rx: RxBundle<D>;
    /// construct all channel pairs
    fn new() -> (Self, Self::Rx);
}

pub trait RxBundle<D>: Sized + Send + 'static {
    /// drains all channels without blocking, routing messages to destination methods
    fn drain_all(&mut self, dest: &mut D);
}

// utility for workers with no auxiliary channels
impl<D> TxBundle<D> for () {
    type Rx = ();
    fn new() -> ((), ()) { ((), ()) }
}

impl<D> RxBundle<D> for () {
    fn drain_all(&mut self, _dest: &mut D) {}
}

#[macro_export]
macro_rules! channel_routing_for {
    (
        $prefix:ident {
            $(
                $source:ident => {
                    $( $field:ident : [ $msg:ty ; $size:expr ] ),* $(,)?
                }
            ),* $(,)?
        }
    ) => {
        paste::paste! {
            pub struct [<$prefix TxBundle>] {
                $(
                    $(pub [<$source:snake _ $field>]: Option<rtrb::Producer<$msg>>,)*
                )*
            }

            pub struct [<$prefix RxBundle>] {
                $(
                    $(pub [<$source:snake _ $field>]: rtrb::Consumer<$msg>,)*
                )*
            }

            $(
                pub struct [<$source To $prefix>] {
                    $(pub $field: rtrb::Producer<$msg>,)*
                }
            )*

            impl [<$prefix TxBundle>] {
                $(
                    pub fn [<take_ $source:snake>](&mut self) -> Option<[<$source To $prefix>]> {
                        Some([<$source To $prefix>] {
                            $(
                                $field: self.[<$source:snake _ $field>].take()?,
                            )*
                        })
                    }
                )*
            }

            impl<W> $crate::sync::chan::TxBundle<W> for [<$prefix TxBundle>]
            where
                W: Sized $( $( + $crate::sync::chan::MessageHandler<$msg> )* )*
            {
                type Rx = [<$prefix RxBundle>];

                fn new() -> (Self, Self::Rx) {
                    $(
                        $(
                            let ([<$source:snake _ $field _tx>], [<$source:snake _ $field _rx>]) = rtrb::RingBuffer::new($size);
                        )*
                    )*
                    (
                        Self { $($([<$source:snake _ $field>]: Some([<$source:snake _ $field _tx>]),)*)* },
                        Self::Rx { $($([<$source:snake _ $field>]: [<$source:snake _ $field _rx>],)*)* },
                    )
                }
            }

            impl<W> $crate::sync::chan::RxBundle<W> for [<$prefix RxBundle>]
            where
                W: Sized $( $( + $crate::sync::chan::MessageHandler<$msg> )* )*
            {
                fn drain_all(&mut self, dest: &mut W) {
                    $(
                        $(
                            while let Ok(msg) = self.[<$source:snake _ $field>].pop() {
                                $crate::sync::chan::MessageHandler::handle(dest, msg);
                            }
                        )*
                    )*
                }
            }
        }
    };
}

pub(crate) use channel_routing_for;
// }}}
