use env::{MemoryFail, MemoryResult, TeamId, ViewId, ViewSelection};

use super::Reference;

/// Mapping control record. The Space owns the mapping; dropping this value does not detach it.
#[must_use]
pub struct View {
    team: TeamId,
    id: Option<ViewId>,
    address: usize,
    size: usize,
    pub(super) pending: Option<Reference>,
}

impl View {
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
        super::set_access(self.team, self.id()?, selection, access)
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
            super::detach(self.team, id, ViewSelection::Whole)?;
            self.id = None;
        }
        self.release_source()
    }
}
