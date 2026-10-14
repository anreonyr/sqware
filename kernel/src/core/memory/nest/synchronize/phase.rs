/// 关闭阶段。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ClosePhase {
    /// 仍接受映射事务。
    Open,
    /// 映射已撤下，等待 shootdown 结算。
    Closing,
    /// 映射与退休资源都已结算。
    Closed,
}

impl ClosePhase {
    pub(crate) fn is_closed(self) -> bool {
        matches!(self, Self::Closed)
    }

    /// 撤销全部映射：等待失效结算。
    pub(crate) fn begin(self) -> Self {
        Self::Closing
    }

    /// shootdown 成功后进入 Closed；其余阶段不变。
    pub(crate) fn settle(self) -> Self {
        match self {
            Self::Closing => Self::Closed,
            _ => self,
        }
    }
}
