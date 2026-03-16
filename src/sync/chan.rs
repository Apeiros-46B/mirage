use std::{sync::{Arc, Condvar, Mutex, OnceLock, atomic::{AtomicBool, Ordering}}, thread, time::Instant};

use rtrb::{Consumer, PopError, Producer, PushError, RingBuffer};

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

pub trait DeadlineRx<T>: Rx<T> {
    // called before usage on the thread that will be parked
    fn use_current_thread(&self) {}

    fn pop_deadline(&mut self, deadline: Instant) -> Option<T>;

    fn drain_deadline<F>(&mut self, deadline: Instant, mut f: F)
    where F: FnMut(T)
    {
        while let Some(msg) = self.pop_deadline(deadline) {
            f(msg);

            while let Ok(extra_msg) = self.pop() {
                f(extra_msg);

                // if processing batched messages times out, abort immediately
                if Instant::now() >= deadline {
                    return;
                }
            }

            // the queue is now empty but the deadline has not yet been reached
            // the `while let` will call recv_deadline again, blocking until timeout
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

// {{{ deadline blocking wrapper for rtrb queues
pub struct DeadlineProducer<T> {
    tx: Producer<T>,
    sleeping: Arc<AtomicBool>,
    consumer: Arc<OnceLock<thread::Thread>>,
}

pub struct DeadlineConsumer<T> {
    rx: Consumer<T>,
    sleeping: Arc<AtomicBool>,
    thread_slot: Arc<OnceLock<thread::Thread>>,
}

pub fn bounded_deadline<T>(capacity: usize)
    -> (DeadlineProducer<T>, DeadlineConsumer<T>)
{
    let (tx, rx) = RingBuffer::new(capacity);
    let sleeping = Arc::new(AtomicBool::new(false));
    let slot = Arc::new(OnceLock::new());
    (
        DeadlineProducer { tx, sleeping: sleeping.clone(), consumer: slot.clone() },
        DeadlineConsumer { rx, sleeping, thread_slot: slot },
    )
}

impl<T: Send + 'static> Tx<T> for DeadlineProducer<T> {
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

impl<T: Send + 'static> Rx<T> for DeadlineConsumer<T> {
    fn pop(&mut self) -> Result<T, PopError> {
        self.rx.pop()
    }
}

impl<T: Send + 'static> DeadlineRx<T> for DeadlineConsumer<T> {
    fn use_current_thread(&self) {
        self.thread_slot.get_or_init(thread::current);
    }

    fn pop_deadline(&mut self, deadline: Instant) -> Option<T> {
        loop {
            if let Ok(msg) = self.rx.pop() {
                return Some(msg);
            }

            let now = Instant::now();
            if now >= deadline {
                return None;
            }

            self.sleeping.store(true, Ordering::Release);

            // re-check because sender might have pushed a message between our first
            // pop and the previous flag update
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

// {{{ oneshot channels
type Oneshot<T> = Arc<(Mutex<Option<T>>, Condvar)>;

pub struct OneshotProducer<T>(Oneshot<T>);
pub struct OneshotConsumer<T>(Oneshot<T>);

pub fn oneshot<T>() -> (OneshotProducer<T>, OneshotConsumer<T>) {
    let inner = Arc::new((Mutex::new(None), Condvar::new()));
    (OneshotProducer(inner.clone()), OneshotConsumer(inner))
}

impl<T> OneshotProducer<T> {
    pub fn send(self, item: T) {
        let (mtx, cvar) = &*self.0;
        let mut guard = mtx.lock().unwrap();
        assert!(guard.is_none(), "OneshotTx::send called twice");
        *guard = Some(item);
        cvar.notify_one();
    }
}

impl<T> OneshotConsumer<T> {
    pub fn recv(self) -> Option<T> {
        let (mtx, cvar) = &*self.0;
        let mut guard = mtx.lock().unwrap();
        guard = cvar.wait_while(guard, |v| v.is_none()).unwrap();
        guard.take()
    }
}
// }}}
