use alloc::vec::Vec;

use crate::core::memory::address::{PageCount, PageIndex};

use super::{residency_entry::ResidencyEntry, slot::Slot, source::FlatSource};

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

    /// 窗口里有没有页正被一次移交持有/预留：那些页此时没有可交出去的承载。
    pub fn busy(&self, index: PageIndex, count: PageCount) -> bool {
        let FlatSource::Managed(slots) = &self.source else {
            return false;
        };
        let end = index.get() + count.get();
        let first = slots.partition_point(|(slot, _)| slot.get() < index.get());
        let last = slots.partition_point(|(slot, _)| slot.get() < end);
        slots[first..last]
            .iter()
            .any(|(_, state)| matches!(state, Slot::Moving { .. } | Slot::Reserved { .. }))
    }
}
