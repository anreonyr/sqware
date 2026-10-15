use std::{cell::RefCell, rc::Weak};

use crate::retention::Failure;

pub struct Accessor {
    pub ram: Weak<RefCell<Vec<u8>>>,
    pub active: bool,
    pub timeout: bool,
    pub pending: usize,
    pub synchronized: usize,
    pub isolated: usize,
}

impl Accessor {
    pub fn write(&self, byte: u8) -> Result<(), Failure> {
        if !self.active {
            return Err(Failure::Closed);
        }
        self.ram.upgrade().ok_or(Failure::Unknown)?.borrow_mut()[0] = byte;
        Ok(())
    }

    pub fn stop(&mut self) -> Result<(), Failure> {
        if self.timeout || self.pending != 0 {
            return Err(Failure::Pending);
        }
        self.active = false;
        Ok(())
    }
}
