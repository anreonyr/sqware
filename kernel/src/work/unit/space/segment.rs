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
    allocated: Vec<(usize, usize)>,
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
        for (i, &(start, len)) in self.allocated.iter().enumerate() {
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
        self.allocated.insert(at, (cursor, size));
        Ok(cursor)
    }

    pub(crate) fn deallocate(&mut self, addr: usize, size: usize) -> bool {
        match self.allocated.iter().position(|&(start, _)| start == addr) {
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
            .any(|&(start, len)| start == addr && len == size)
    }
}
