//! 一个持久 Domain：实际 Driver 主人、requester、私有 IOVA 游标、引用账与当前绑定。
//!
//! Domain 是唯一总管：没有第二本 Domain 表，也没有域外的绑定影子账。私有游标一旦
//! 为一次绑定分配就不再回退；同 Domain 内已分配的 IOVA 段不会复用。

use env::TaskId;

use super::binding::Binding;
use super::domain_id::DomainId;
use super::domain_range::DomainRange;
use super::hold::Holds;

/// 私有 IOVA 空间从 8 GiB 起。
pub const FIRST_IOVA: u64 = 1 << 33;
/// 每次分配占一个 2 MiB 槽。
pub const IOVA_STRIDE: u64 = 1 << 21;
/// 私有 IOVA 空间的排他上界。
pub const IOVA_LIMIT: u64 = 1 << 38;

pub struct Domain {
    pub(super) id: DomainId,
    pub(super) owner: TaskId,
    pub(super) requester: u16,
    /// 第一次完整绑定号带来的 Hub；此后不能换。
    pub(super) hub: Option<TaskId>,
    pub(super) next_iova: u64,
    /// 已结清绑定号的水位：旧号不能推进新绑定。
    pub(super) retired: u64,
    pub(super) holds: Holds,
    pub(super) binding: Option<Binding>,
}

impl Domain {
    pub(super) fn new(id: DomainId, owner: TaskId, requester: u16) -> Self {
        Self {
            id,
            owner,
            requester,
            hub: None,
            next_iova: FIRST_IOVA,
            retired: 0,
            holds: Holds::new(),
            binding: None,
        }
    }

    pub const fn id(&self) -> DomainId {
        self.id
    }

    pub fn range(&self) -> Option<DomainRange> {
        self.binding.as_ref().map(|binding| binding.range)
    }
}
