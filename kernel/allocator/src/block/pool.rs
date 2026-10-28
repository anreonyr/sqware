use core::ptr::NonNull;

use alloc::vec::Vec;

use crate::InitError;

use super::MAX_POWER;

pub(super) struct Pool {
    pub(super) freepool: Vec<Option<NonNull<u8>>>,
    pub(super) spare: [Option<usize>; MAX_POWER + 1],
}

// SAFETY: 裸指针只是同一块受池锁保护的物理页内的空闲链地址。
unsafe impl Send for Pool {}

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
