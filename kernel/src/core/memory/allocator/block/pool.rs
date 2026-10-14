use core::ptr::NonNull;

use alloc::vec::Vec;

use crate::core::memory::allocator::InitError;

use super::MAX_POWER;

pub(super) struct Pool {
    pub(super) freepool: Vec<Option<NonNull<u8>>>,
    pub(super) spare: [Option<usize>; MAX_POWER + 1],
}

impl Pool {
    pub(super) fn new() -> Self {
        Self {
            freepool: Vec::new(),
            spare: [None; MAX_POWER + 1],
        }
    }

    pub(super) fn init(&mut self) -> Result<(), InitError> {
        self.freepool
            .try_reserve(MAX_POWER + 1)
            .map_err(|_| InitError::OutOfMemory)?;
        self.freepool.resize_with(MAX_POWER + 1, || None);
        Ok(())
    }
}
