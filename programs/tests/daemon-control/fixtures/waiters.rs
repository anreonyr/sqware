use alloc::vec::Vec;
use core::marker::PhantomData;
use ipc::rpc::reply::Sender;
use system_api::control::frame::Said;

pub struct Waiters {
    items: Vec<Sender<Said>>,
    capacity: usize,
    marker: PhantomData<Said>,
}

impl Waiters {
    pub fn empty(capacity: usize) -> Self {
        Self {
            items: Vec::new(),
            capacity,
            marker: PhantomData,
        }
    }

    pub fn with_capacity(capacity: usize) -> Self {
        Self {
            items: Vec::new(),
            capacity,
            marker: PhantomData,
        }
    }

    pub fn try_reserve(&mut self, additional: usize) -> Result<(), ()> {
        if self.items.len() + additional <= self.capacity {
            Ok(())
        } else {
            Err(())
        }
    }

    pub fn push(&mut self, sender: Sender<Said>) {
        self.items.push(sender);
    }

    pub fn len(&self) -> usize {
        self.items.len()
    }
}

impl Default for Waiters {
    fn default() -> Self {
        Self::empty(usize::MAX)
    }
}

impl IntoIterator for Waiters {
    type Item = Sender<Said>;
    type IntoIter = alloc::vec::IntoIter<Sender<Said>>;

    fn into_iter(self) -> Self::IntoIter {
        self.items.into_iter()
    }
}
