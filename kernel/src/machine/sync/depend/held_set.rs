use alloc::vec::Vec;

use super::error::DepInitError;
use super::held::Held;
use super::level::Level;

const MAX_HELD: usize = 8;

pub(super) struct HeldSet {
    pub(super) slots: Vec<Held>,
}

impl HeldSet {
    pub(super) fn new() -> Result<HeldSet, DepInitError> {
        let mut slots = Vec::new();
        slots
            .try_reserve_exact(MAX_HELD)
            .map_err(|_| DepInitError::OutOfMemory)?;
        Ok(HeldSet { slots })
    }

    pub(super) fn max_level(&self) -> Option<Level> {
        self.slots.iter().filter_map(|h| h.level).max()
    }

    pub(super) fn contains(&self, addr: usize) -> bool {
        self.slots.iter().any(|h| h.addr == addr)
    }

    pub(super) fn push(&mut self, addr: usize, level: Option<Level>, caller: usize) -> Result<(), ()> {
        if self.slots.len() >= self.slots.capacity() {
            return Err(());
        }
        self.slots.push(Held {
            addr,
            level,
            caller,
        });
        Ok(())
    }

    pub(super) fn remove(&mut self, addr: usize) -> Result<(), ()> {
        for i in 0..self.slots.len() {
            if self.slots[i].addr == addr {
                self.slots.remove(i);
                return Ok(());
            }
        }
        Err(())
    }
}
