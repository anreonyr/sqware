use alloc::sync::Arc;
use core::sync::atomic::{AtomicBool, Ordering};
use env::{PieFail, PieToken, TaskId};

use crate::grant::Grant;
use crate::lock::SpinLock;
use crate::memory::address::PageCount;
use crate::memory::flat::FlatRange;
use crate::pend::SourceAuthorization;
use crate::work::mail::OperationState;

pub struct PoleMeta {
    owner: TaskId,
    live: AtomicBool,
    pub(crate) control: Arc<OperationState>,
    source: FlatRange,
    authorization: SpinLock<Option<Arc<SourceAuthorization>>>,
}

impl PoleMeta {
    pub fn new(owner: TaskId) -> Arc<Self> {
        Self::with_source(owner, FlatRange::new(PageCount::new(8).unwrap()))
    }

    pub fn with_source(owner: TaskId, source: FlatRange) -> Arc<Self> {
        Arc::new(Self {
            owner,
            live: AtomicBool::new(true),
            control: OperationState::new(),
            source,
            authorization: SpinLock::new(None),
        })
    }

    pub fn owner(&self) -> TaskId {
        self.owner
    }

    pub fn alive(&self) -> bool {
        self.live.load(Ordering::Acquire)
    }

    pub fn reserved(&self) -> usize {
        self.control.reserved()
    }

    pub fn has_cells(&self) -> bool {
        false
    }

    pub fn seal(&self) {
        self.live.store(false, Ordering::Release);
    }

    pub fn stage_for(&self, team: usize) {
        self.control.stage_for(team);
    }

    pub fn source(&self) -> FlatRange {
        self.source.clone()
    }

    pub fn authorization(&self) -> Option<Arc<SourceAuthorization>> {
        self.authorization.lock().clone()
    }

    pub fn attach_source(&self, source: &Arc<SourceAuthorization>) {
        *self.authorization.lock() = Some(source.clone());
    }

    pub fn withdraw(&self, _: PieToken) {}

    pub fn release_grant(&self, _: PieToken) {}

    pub fn register_grant(&self, _: PieToken, _: Arc<Grant>) -> Result<(), PieFail> {
        Ok(())
    }
}
