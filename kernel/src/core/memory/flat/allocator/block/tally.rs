use crate::core::memory::PAGE_SIZE;
use crate::machine::sync::{Level, SpinLock};

use super::PAGE_SHIFT;
use super::cell::Cell;

pub(super) struct Tally {
    base: usize,
    cells: *mut Cell,
    len: usize,
    lock: SpinLock<()>,
}

unsafe impl Send for Tally {}
unsafe impl Sync for Tally {}

impl Tally {
    pub(super) fn new(base: usize, cells: *mut Cell, len: usize) -> Self {
        Self {
            base,
            cells,
            len,
            lock: SpinLock::new_level(Level::Tally, ()),
        }
    }

    fn idx(&self, pa: usize) -> Option<usize> {
        let page = pa & !(PAGE_SIZE - 1);
        let idx = page.checked_sub(self.base)? >> PAGE_SHIFT;
        (idx < self.len).then_some(idx)
    }

    pub(super) fn frame_of(&self, idx: usize) -> usize {
        self.base + (idx << PAGE_SHIFT)
    }

    pub(super) fn len(&self) -> usize {
        self.len
    }

    pub(super) fn owner_of(&self, pa: usize) -> Option<usize> {
        let _g = self.lock.lock();
        let m = unsafe { self.cells.add(self.idx(pa)?).read() };
        m.owner()
    }

    pub(super) fn read_idx(&self, idx: usize) -> Cell {
        let _g = self.lock.lock();
        unsafe { self.cells.add(idx).read() }
    }

    pub(super) fn write(&self, page: usize, m: Cell) {
        let _g = self.lock.lock();
        let idx = self.idx(page).expect("block tally: page out of table");
        unsafe {
            self.cells.add(idx).write(m);
        }
    }

    pub(super) fn inc_used(&self, page: usize) {
        let _g = self.lock.lock();
        let idx = self.idx(page).expect("block tally: page out of table");
        unsafe {
            let m = self.cells.add(idx).read().inc_used();
            self.cells.add(idx).write(m);
        }
    }

    pub(super) fn dec_used(&self, page: usize) -> bool {
        let _g = self.lock.lock();
        let idx = self.idx(page).expect("block tally: page out of table");
        unsafe {
            let (m, empty) = self.cells.add(idx).read().dec_used();
            self.cells.add(idx).write(m);
            empty
        }
    }
}
