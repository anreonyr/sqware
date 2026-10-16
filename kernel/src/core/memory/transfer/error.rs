use fack::prelude::Error;

/// 无映射移交的纯域错误：不认识线上编码。
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
    #[error("this carrier is not a plain acquired page")]
    NotTransferable,
    #[error("the page slot or its residency is held by another use")]
    Busy,
}
