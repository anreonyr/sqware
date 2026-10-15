use alloc::vec::Vec;

use crate::core::memory::address::{PageCount, PageIndex};

use super::{residency_entry::ResidencyEntry, source::FlatSource};

pub(super) struct FlatState {
    pub source: FlatSource,
    pub residencies: Vec<ResidencyEntry>,
    pub next_residency: usize,
}

impl FlatState {
    pub fn new(source: FlatSource) -> Self {
        Self {
            source,
            residencies: Vec::new(),
            next_residency: 1,
        }
    }

    pub fn is_pinned(&self, index: PageIndex, count: PageCount) -> bool {
        let end = index.get() + count.get();
        self.residencies.iter().any(|entry| {
            entry.index.get() < end && index.get() < entry.index.get() + entry.count.get()
        })
    }
}
