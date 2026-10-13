use alloc::boxed::Box;

use super::reclaim::Reclaim;

/// 待清理链条上的一环：要么记一笔映射，要么记一段撤回。
pub(super) struct CleanupNode {
    pub(super) record: Option<Reclaim>,
    pub(super) next: Option<Box<CleanupNode>>,
}

impl CleanupNode {
    pub(super) const fn empty() -> Self {
        Self {
            record: None,
            next: None,
        }
    }
}
