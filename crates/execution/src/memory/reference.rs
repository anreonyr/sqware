use env::{
    AcquireSource, FlatReference, MemoryFail, MemoryResult, PieToken, Placement, ReferenceRange,
    TeamId, ViewControl,
};

use super::{PAGE_SIZE, View};

/// Task-scoped source reference. Release is explicit; a failed release keeps the handle.
#[must_use]
pub struct Reference {
    handle: Option<FlatReference>,
}

impl Reference {
    pub fn acquire(source: AcquireSource, retention: PieToken) -> MemoryResult<Self> {
        super::acquire(source, retention).map(|handle| Self {
            handle: Some(handle),
        })
    }

    pub fn handle(&self) -> MemoryResult<FlatReference> {
        self.handle
            .ok_or_else(|| env::make_fail(MemoryFail::Denied))
    }

    pub fn release(&mut self) -> MemoryResult<()> {
        super::release(self.handle()?)?;
        self.handle = None;
        Ok(())
    }

    pub fn attach(
        &self,
        team: TeamId,
        offset: usize,
        count: usize,
        placement: Placement,
        access: u64,
        control: ViewControl,
    ) -> MemoryResult<View> {
        let range = ReferenceRange::new(self.handle()?, offset, count)
            .ok_or_else(|| env::make_fail(MemoryFail::NotAligned))?;
        let size = count
            .checked_mul(PAGE_SIZE)
            .ok_or_else(|| env::make_fail(MemoryFail::NotAligned))?;
        let (id, address) = super::attach(range, team, placement, access, control)?;
        Ok(View::new(team, id, address, size))
    }
}
