// TODO: user-friendly cross-platform fatal crash dialogs

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
