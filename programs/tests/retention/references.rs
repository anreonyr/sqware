use env::{AcquireSource, FlatReference, PieToken};
use std::{
    cell::RefCell,
    collections::BTreeMap,
    rc::{Rc, Weak},
};

use crate::retention::{Failure, MemoryEffects};

pub struct References {
    pub source: Weak<RefCell<Vec<u8>>>,
    pub held: BTreeMap<usize, Rc<RefCell<Vec<u8>>>>,
    pub next: usize,
    pub fail_acquire: bool,
    pub fail_release: bool,
}

impl MemoryEffects for References {
    fn acquire(&mut self, _: AcquireSource, retention: PieToken) -> Result<FlatReference, Failure> {
        if self.fail_acquire || retention.get() != 3 {
            return Err(Failure::Denied);
        }
        let ram = self.source.upgrade().ok_or(Failure::Denied)?;
        let id = self.next;
        self.next += 1;
        self.held.insert(id, ram);
        Ok(FlatReference::from_bytes(&id.to_le_bytes()).unwrap())
    }

    fn release(&mut self, reference: FlatReference) -> Result<(), Failure> {
        if self.fail_release {
            return Err(Failure::Pending);
        }
        self.held.remove(&reference.get()).ok_or(Failure::Denied)?;
        Ok(())
    }
}
