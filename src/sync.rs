use std::sync::{Arc, Condvar, Mutex};

use rtrb::Producer;
use winit::window::Window;

pub struct RenderInit {
    pub win: Arc<Window>,
}

pub enum RenderMsg {
    Resized,
    Exiting,
}

pub struct AppChannels {
    app_to_render: Producer<RenderMsg>,
}

type Oneshot<T> = Arc<(Mutex<Option<T>>, Condvar)>;

pub struct OneshotTx<T>(Oneshot<T>);
pub struct OneshotRx<T>(Oneshot<T>);

pub fn oneshot<T>() -> (OneshotTx<T>, OneshotRx<T>) {
    let inner = Arc::new((Mutex::new(None), Condvar::new()));
    (OneshotTx(inner.clone()), OneshotRx(inner))
}

impl<T> OneshotTx<T> {
    pub fn send(self, item: T) {
        let (mtx, cvar) = &*self.0;
        let mut guard = mtx.lock().unwrap();
        assert!(guard.is_none(), "OneshotTx::send called twice");
        *guard = Some(item);
        cvar.notify_one();
    }
}

impl<T> OneshotRx<T> {
    pub fn recv(self) -> Option<T> {
        let (mtx, cvar) = &*self.0;
        let mut guard = mtx.lock().unwrap();
        guard = cvar.wait_while(guard, |v| v.is_none()).unwrap();
        guard.take()
    }
}
