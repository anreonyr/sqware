use core::marker::PhantomData;

use env::{
    AcquireSource, FlatReference, MemoryFail, MemoryResult, PieToken, Placement, ReferenceRange,
    TeamId, ViewControl,
};

use super::PAGE_SIZE;
use super::mapping_kernel::MappingKernel;
use super::reference_kernel::ReferenceKernel;
use super::view::View;

/// Task-scoped source reference. Release is explicit; a failed release keeps the handle.
#[must_use]
pub struct Reference<K: ReferenceKernel> {
    handle: Option<FlatReference>,
    _kernel: PhantomData<fn() -> K>,
}

impl<K: ReferenceKernel> Reference<K> {
    pub fn acquire(source: AcquireSource, retention: PieToken) -> MemoryResult<Self> {
        K::acquire(source, retention).map(|handle| Self {
            handle: Some(handle),
            _kernel: PhantomData,
        })
    }

    pub fn handle(&self) -> MemoryResult<FlatReference> {
        self.handle
            .ok_or_else(|| env::make_fail(MemoryFail::Denied))
    }

    pub fn release(&mut self) -> MemoryResult<()> {
        K::release(self.handle()?)?;
        self.handle = None;
        Ok(())
    }
}

impl<K: ReferenceKernel + MappingKernel> Reference<K> {
    pub fn attach(
        &self,
        team: TeamId,
        offset: usize,
        count: usize,
        placement: Placement,
        access: u64,
        control: ViewControl,
    ) -> MemoryResult<View<K>> {
        let range = ReferenceRange::new(self.handle()?, offset, count)
            .ok_or_else(|| env::make_fail(MemoryFail::NotAligned))?;
        let size = count
            .checked_mul(PAGE_SIZE)
            .ok_or_else(|| env::make_fail(MemoryFail::NotAligned))?;
        let (id, address) = K::attach(range, team, placement, access, control)?;
        Ok(View::new(team, id, address, size))
    }
}
