use env::{MemoryFail, MemoryResult, TeamId, ViewId, ViewSelection};

use super::mapping_kernel::MappingKernel;
use super::reference::Reference;
use super::reference_kernel::ReferenceKernel;

/// Mapping control record. The Space owns the mapping; dropping this value does not detach it.
#[must_use]
pub struct View<K: ReferenceKernel + MappingKernel> {
    team: TeamId,
    id: Option<ViewId>,
    address: usize,
    size: usize,
    pub(super) pending: Option<Reference<K>>,
}

impl<K: ReferenceKernel + MappingKernel> View<K> {
    pub(super) fn new(team: TeamId, id: ViewId, address: usize, size: usize) -> Self {
        Self {
            team,
            id: Some(id),
            address,
            size,
            pending: None,
        }
    }

    pub fn id(&self) -> MemoryResult<ViewId> {
        self.id.ok_or_else(|| env::make_fail(MemoryFail::Denied))
    }

    pub fn address(&self) -> usize {
        self.address
    }

    pub fn size(&self) -> usize {
        self.size
    }

    pub fn set_access(&self, selection: ViewSelection, access: u64) -> MemoryResult<()> {
        K::set_access(self.team, self.id()?, selection, access)
    }

    pub fn release_source(&mut self) -> MemoryResult<()> {
        if let Some(reference) = &mut self.pending {
            reference.release()?;
            self.pending = None;
        }
        Ok(())
    }

    pub fn close(&mut self) -> MemoryResult<()> {
        if let Some(id) = self.id {
            K::detach(self.team, id, ViewSelection::Whole)?;
            self.id = None;
        }
        self.release_source()
    }
}
