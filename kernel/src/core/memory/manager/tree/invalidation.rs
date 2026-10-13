/// 失效状态机。
///
/// `Fencing` 期间等待者只等结果，不得看到旧失败。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Invalidation {
    /// 无待失效的 PTE 修改。
    Clean,
    /// 有已发布的 PTE 修改，等待 shootdown。
    Dirty,
    /// shootdown 进行中。
    Fencing,
    /// shootdown 失败：待失效标记与退休资源都保留，可重试。
    Failed,
}

impl Invalidation {
    pub(super) fn is_clean(self) -> bool {
        matches!(self, Self::Clean)
    }

    pub(super) fn is_fencing(self) -> bool {
        matches!(self, Self::Fencing)
    }

    /// 记录一次 PTE 修改：回到待失效。
    pub(super) fn mark_dirty(self) -> Self {
        Self::Dirty
    }

    /// 开始（或重试）shootdown。
    pub(super) fn begin(self) -> Self {
        Self::Fencing
    }

    /// shootdown 成功：待失效资源可以结算。
    pub(super) fn complete(self) -> Self {
        Self::Clean
    }

    /// shootdown 失败：保留待失效标记与资源，等待重试。
    pub(super) fn fail(self) -> Self {
        Self::Failed
    }
}
