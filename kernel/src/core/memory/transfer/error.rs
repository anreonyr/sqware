use fack::prelude::Error;

/// 有映射移交的纯域错误：不认识线上编码。
#[derive(Error, Debug, Clone, Copy, PartialEq, Eq)]
pub enum TransferError {
    #[error("physical frame allocator exhausted")]
    OutOfMemory,
    #[error("page selection falls outside the source range")]
    OutOfRange,
    #[error("the source page holds no carrier to hand over")]
    SourceEmpty,
    #[error("the destination page slot is not empty")]
    DestinationOccupied,
    /// 永久拒结：承载不是普通 `acquire` 页（导入/借用），或它的旧映射无从核实
    /// （属主失联、内核永久映射）。这类拒绝重试也不会变。
    #[error("this page cannot be handed over")]
    NotTransferable,
    #[error("the page slot, its residency, or an unconfirmed sync is held by another use")]
    Busy,
}
