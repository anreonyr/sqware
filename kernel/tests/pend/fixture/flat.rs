use alloc::sync::Arc;

use crate::memory::address::{PageCount, PageIndex, PageOffset};

#[path = "residency.rs"]
mod residency;
pub use residency::Residency;

#[derive(Clone)]
pub struct FlatRange {
    identity: Arc<()>,
    index: PageIndex,
    count: PageCount,
}

impl FlatRange {
    pub fn new(count: PageCount) -> Self {
        Self {
            identity: Arc::new(()),
            index: PageIndex::ZERO,
            count,
        }
    }

    pub fn index(&self) -> PageIndex {
        self.index
    }

    pub fn page_count(&self) -> PageCount {
        self.count
    }

    pub fn reside(&self) -> Result<Residency, ()> {
        Ok(Residency::new(self.clone()))
    }

    pub fn range(&self, offset: PageOffset, count: PageCount) -> Result<Self, ()> {
        if offset
            .get()
            .checked_add(count.get())
            .is_none_or(|end| end > self.count.get())
        {
            return Err(());
        }
        Ok(Self {
            identity: self.identity.clone(),
            index: PageIndex::new(self.index.get().checked_add(offset.get()).ok_or(())?),
            count,
        })
    }
}
