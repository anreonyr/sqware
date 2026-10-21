use alloc::sync::Arc;
use alloc::vec::Vec;
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
    /// 这份资源名下的授权记录：与内核那侧同一口径——活与授权由这两项一起说。
    authorities: SpinLock<Vec<(PieToken, Arc<Grant>)>>,
    /// host 假机制：只把「这份 Pole 账上有一条关系」这一条事实说出来，
    /// 不为它造 View / Nest。
    mapped: AtomicBool,
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
            authorities: SpinLock::new(Vec::new()),
            mapped: AtomicBool::new(false),
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

    pub fn has_maps(&self) -> bool {
        self.mapped.load(Ordering::Acquire)
    }

    /// 置「这份账上有一条关系」：host 只记这一条事实。
    pub fn note_map(&self) {
        self.mapped.store(true, Ordering::Release);
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

    /// 登记一枚授权：与内核那侧同一口径，记的就是 token 与它当前那枚 Grant。
    pub fn register_grant(&self, token: PieToken, grant: Arc<Grant>) -> Result<(), PieFail> {
        self.authorities.lock().push((token, grant));
        Ok(())
    }

    /// 撇下一枚授权记录（`Pie` 退场走这一手）。
    pub fn release_grant(&self, token: PieToken) {
        self.authorities.lock().retain(|(held, _)| *held != token);
    }

    /// 还有可用的授权：这份资源自己活着，且名下至少一枚授权记录没失效。
    pub fn authorizing(&self) -> bool {
        self.live.load(Ordering::Acquire)
            && self
                .authorities
                .lock()
                .iter()
                .any(|(_, grant)| grant.permission().is_some())
    }
}
