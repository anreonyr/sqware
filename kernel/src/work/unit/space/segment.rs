use alloc::alloc::AllocError;
use alloc::vec::Vec;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum SegmentKind {
    Normal,
    Kernel,
}

#[derive(Debug)]
pub(crate) struct Segment {
    base: usize,
    edge: usize,
    allocated: Vec<(usize, usize, usize)>,
}

impl Segment {
    pub(crate) fn new(base: usize, edge: usize) -> Self {
        Self {
            base,
            edge,
            allocated: Vec::new(),
        }
    }

    pub(crate) fn allocate(&mut self, size: usize) -> Result<usize, AllocError> {
        let size = size.max(1);
        let mut cursor = self.base;
        let mut at = self.allocated.len();
        for (i, &(start, len, _)) in self.allocated.iter().enumerate() {
            if start.saturating_sub(cursor) >= size {
                at = i;
                break;
            }
            cursor = cursor.max(start.saturating_add(len));
        }
        if at == self.allocated.len() {
            if self.edge.saturating_sub(cursor) < size {
                return Err(AllocError);
            }
        }
        self.allocated.try_reserve(1).map_err(|_| AllocError)?;
        self.allocated.insert(at, (cursor, size, 0));
        Ok(cursor)
    }

    pub(crate) fn reserve(&mut self, addr: usize, size: usize) -> bool {
        let Some(end) = addr.checked_add(size) else {
            return false;
        };
        if size == 0 || addr < self.base || end > self.edge {
            return false;
        }
        let at = self
            .allocated
            .partition_point(|&(start, _, _)| start < addr);
        if at > 0 && self.allocated[at - 1].0 + self.allocated[at - 1].1 > addr
            || at < self.allocated.len() && end > self.allocated[at].0
        {
            return false;
        }
        if self.allocated.try_reserve(1).is_err() {
            return false;
        }
        self.allocated.insert(at, (addr, size, 0));
        true
    }

    pub(crate) fn deallocate(&mut self, addr: usize, size: usize) -> bool {
        match self
            .allocated
            .iter()
            .position(|&(start, _, retired)| start == addr && retired == 0)
        {
            Some(at) if self.allocated[at].1 == size => {
                self.allocated.remove(at);
                true
            }
            _ => false,
        }
    }

    pub(crate) fn holds(&self, addr: usize, size: usize) -> bool {
        self.allocated
            .iter()
            .any(|&(start, len, retired)| start == addr && len == size && retired == 0)
    }

    pub(crate) fn prepare_cut(&mut self) -> Result<(), AllocError> {
        self.allocated.try_reserve(2).map_err(|_| AllocError)
    }

    pub(crate) fn covering(&self, addr: usize) -> Option<(usize, usize)> {
        self.allocated.iter().find_map(|&(start, len, retired)| {
            (retired == 0 && start <= addr && addr < start + len).then_some((start, len))
        })
    }

    /// Keep removed addresses reserved until their PTE eviction completes.
    pub(crate) fn retire(&mut self, addr: usize, size: usize) -> usize {
        use core::sync::atomic::{AtomicUsize, Ordering};
        static NEXT: AtomicUsize = AtomicUsize::new(1);
        let ticket = NEXT.fetch_add(1, Ordering::Relaxed);
        assert!(ticket != 0, "segment retirement exhausted");
        let end = addr + size;
        let mut i = 0;
        while i < self.allocated.len() {
            let (start, len, retired) = self.allocated[i];
            let stop = start + len;
            if retired != 0 || addr >= stop || end <= start {
                i += 1;
                continue;
            }
            self.allocated.remove(i);
            if start < addr {
                self.allocated.insert(i, (start, addr - start, 0));
                i += 1;
            }
            let lo = start.max(addr);
            let hi = stop.min(end);
            self.allocated.insert(i, (lo, hi - lo, ticket));
            i += 1;
            if stop > end {
                self.allocated.insert(i, (end, stop - end, 0));
                i += 1;
            }
        }
        ticket
    }

    pub(crate) fn reclaim(&mut self, ticket: usize) {
        self.allocated.retain(|&(_, _, retired)| retired != ticket);
    }
}
