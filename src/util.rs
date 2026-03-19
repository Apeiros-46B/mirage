// TODO: user-friendly cross-platform fatal crash dialogs

use std::mem::MaybeUninit;

pub struct HistoryRing<T> {
    pub data: Vec<T>,
    pub head: usize,
}

impl<T: Copy> HistoryRing<T> {
    pub fn new(capacity: usize, default: T) -> Self {
        let mut data = Vec::with_capacity(capacity);
        data.resize(capacity, default);
        Self {
            data,
            head: 0,
        }
    }

    pub fn write(&mut self, val: T) {
        self.data[self.head] = val;
        self.head = (self.head + 1) % self.data.len();
    }

    pub fn iter_head(&self, default: T) -> impl Iterator<Item = T> {
        self.data.iter().enumerate().map(move |(i, v)| {
            if i == self.head {
                default
            } else {
                *v
            }
        })
    }
}

/// reimplementation of `std::array::try_from_fn` (since it is not yet stable)
pub fn array_try_from_fn<T, E, F, const N: usize>(mut cb: F) -> Result<[T; N], E>
where
    F: FnMut(usize) -> Result<T, E>,
{
    // safety: calling assume_init() on an array of MaybeUninit is sound because
    // MaybeUninit itself does not require initialization
    let mut array: [MaybeUninit<T>; N] = unsafe {
        MaybeUninit::uninit().assume_init()
    };

    for i in 0..N {
        match cb(i) {
            Ok(value) => {
                array[i].write(value);
            }
            Err(err) => {
                // if cb(1) succeeds but cb(2) fails, we must manually drop the previously
                // successfully initialized items before returning the error
                for j in 0..i {
                    unsafe {
                        array[j].assume_init_drop();
                    }
                }
                return Err(err);
            }
        }
    }

    // safe because the loop above guarantees all N elements have been written and
    // because [MaybeUninit<T>; N] has the same memory layout as [T; N]
    let ptr = array.as_ptr() as *const [T; N];
    Ok(unsafe { std::ptr::read(ptr) })
}
