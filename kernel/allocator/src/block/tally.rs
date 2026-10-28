use crate::lock::{Lock, LockRole};
use crate::Locked;
use env::PAGE_SIZE;

use super::PAGE_SHIFT;
use super::cell::Cell;

pub(super) struct Tally<L: Lock> {
    base: usize,
    cells: *mut Cell,
    len: usize,
    guard: Locked<L, ()>,
}

// SAFETY: `cells` 指向 tally 自己的簿记表，所有读写都经过 `guard` 的原生锁。
unsafe impl<L: Lock> Sync for Tally<L> {}

impl<L: Lock> Tally<L> {
    pub(super) fn new(base: usize, cells: *mut Cell, len: usize) -> Self {
        Self {
            base,
            cells,
            len,
            guard: Locked::new(L::new(Some(LockRole::Tally)), ()),
        }
    }

    fn with<R>(&self, f: impl FnOnce() -> R) -> R {
        self.guard.with(|()| f())
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
        self.with(|| {
            let m = unsafe { self.cells.add(self.idx(pa)?).read() };
            m.owner()
        })
    }

    pub(super) fn read_idx(&self, idx: usize) -> Cell {
        self.with(|| unsafe { self.cells.add(idx).read() })
    }

    pub(super) fn write(&self, page: usize, m: Cell) {
        self.with(|| {
            let idx = self.idx(page).expect("block tally: page out of table");
            unsafe {
                self.cells.add(idx).write(m);
            }
        })
    }

    pub(super) fn inc_used(&self, page: usize) {
        self.with(|| {
            let idx = self.idx(page).expect("block tally: page out of table");
            unsafe {
                let m = self.cells.add(idx).read().inc_used();
                self.cells.add(idx).write(m);
            }
        })
    }

    pub(super) fn dec_used(&self, page: usize) -> bool {
        self.with(|| {
            let idx = self.idx(page).expect("block tally: page out of table");
            unsafe {
                let (m, empty) = self.cells.add(idx).read().dec_used();
                self.cells.add(idx).write(m);
                empty
            }
        })
    }
}
