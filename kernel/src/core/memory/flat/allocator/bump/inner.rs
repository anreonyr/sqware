use crate::core::memory::flat::allocator::InitError;
use crate::machine::board;

pub(super) struct BumpInner {
    pub(super) used: usize,
    pub(super) base: usize,
    pub(super) edge: usize,
}

impl BumpInner {
    pub(super) const fn new(base: usize, edge: usize, used: usize) -> Self {
        Self { base, edge, used }
    }

    pub(super) fn init(&mut self) -> Result<(), InitError> {
        let m = board::info();
        if m.free.size == 0 {
            return Err(InitError::NoFreeMemory);
        }
        self.base = m.free.base;
        self.edge = m.free.base + m.free.size;
        Ok(())
    }
}
