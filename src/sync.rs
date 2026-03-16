use std::{sync::{Arc, Condvar, Mutex, atomic::{AtomicBool, Ordering}}, thread::{self, JoinHandle}, time::Instant};

use rtrb::{Consumer, Producer, RingBuffer};

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

// {{{ utility for managing persistent worker threads
pub trait Worker where Self: Sized {
    const NAME: &'static str;

    type Init: Send + 'static;
    type Msg: Send + Sync + 'static;

    fn new(init: Self::Init) -> anyhow::Result<Self>;

    fn poll(&mut self, time: f64, dt: f32, tick_idx: usize) -> anyhow::Result<()>;

    fn handle(&mut self, msg: Self::Msg);

    fn run(mut self, mut rx: Consumer<Self::Msg>, should_exit: Arc<AtomicBool>) {
        let start_time = Instant::now();
        let mut prev_tick_time = start_time;
        let mut tick_idx = 0;

        loop {
            if should_exit.load(Ordering::Relaxed) {
                return;
            }

            while let Ok(msg) = rx.pop() {
                self.handle(msg);
            }

            let now = Instant::now();
            let time = (now - start_time).as_secs_f64();
            let dt = (now - prev_tick_time).as_secs_f32();

            if let Err(err) = self.poll(time, dt, tick_idx) {
                eprintln!("error in {} at local tick #{}: {}", Self::NAME, tick_idx, err);
            }

            prev_tick_time = now;
            tick_idx += 1;
        }
    }

    fn spawn() -> WorkerHandle<Self> {
        let should_exit = Arc::new(AtomicBool::new(false));
        let should_exit_moved = should_exit.clone();

        let (itx, irx) = oneshot();
        let (tx, rx) = RingBuffer::new(256);
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
    tx: Producer<W::Msg>,
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

    /// signals the worker to stop and blocks until the thread finishes its current tick
    pub fn exit(&mut self) {
        self.should_exit.store(true, Ordering::Relaxed);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}
// }}}
