use fack::prelude::Error;

/// 物理来源的纯域错误：不认识线上编码。
#[derive(Error, Debug, Clone, Copy, PartialEq, Eq)]
pub enum FlatError {
    #[error("physical frame allocator exhausted")]
    OutOfMemory,
    #[error("physical allocation constraint is not supported")]
    NotAligned,
    #[error("page selection falls outside the source range")]
    OutOfRange,
    #[error("the page carrier has been moved out")]
    Missing,
    #[error("the page slot is held by another use")]
    Busy,
}
