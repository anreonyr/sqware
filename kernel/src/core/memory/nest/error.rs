use fack::prelude::Error;

use crate::core::memory::flat::FlatError;

/// 地址空间的纯域错误：只讲地址空间自己的失败，不认识线上编码。
#[derive(Error, Debug, Clone, Copy, PartialEq, Eq)]
pub enum NestError {
    #[error("physical frame allocator exhausted")]
    OutOfMemory,
    #[error("virtual address already mapped")]
    AlreadyMapped,
    #[error("address not page-aligned")]
    NotAligned,
    #[error("page table entry not mapped")]
    NotMapped,
    #[error("virtual address not in any declared map")]
    NoRegion,
    #[error("DRAM identity map exceeds the lower address half")]
    DramOverlap,
    #[error("cannot widen a borrowed mapping")]
    WidenDenied,
    #[error("address-space shootdown failed")]
    Shootdown,
    #[error("kernel address space has permanent lifetime")]
    Pinned,
}

/// 物理来源失败折入地址空间失败码：`OutOfRange` 对应用户可见的"没有所属区域"。
impl From<FlatError> for NestError {
    fn from(error: FlatError) -> Self {
        match error {
            FlatError::OutOfMemory => NestError::OutOfMemory,
            FlatError::NotAligned => NestError::NotAligned,
            FlatError::OutOfRange => NestError::NoRegion,
        }
    }
}
