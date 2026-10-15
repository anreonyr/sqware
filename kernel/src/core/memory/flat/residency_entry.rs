use crate::core::memory::address::{PageCount, PageIndex};

pub(super) struct ResidencyEntry {
    pub id: usize,
    pub index: PageIndex,
    pub count: PageCount,
}
