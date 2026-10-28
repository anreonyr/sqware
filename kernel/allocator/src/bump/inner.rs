use crate::{InitError, Region};

pub(super) struct BumpInner {
    pub(super) used: usize,
    pub(super) base: usize,
    pub(super) edge: usize,
}

impl BumpInner {
    pub(super) const fn new(base: usize, edge: usize, used: usize) -> Self {
        Self { base, edge, used }
    }

    pub(super) fn init(&mut self, free: Region) -> Result<(), InitError> {
        if free.size == 0 {
            return Err(InitError::NoFreeMemory);
        }
        self.base = free.base;
        self.edge = free.base + free.size;
        Ok(())
    }
}
