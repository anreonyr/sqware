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
    /// 撤不掉也确认不了：内核地址空间是进程期的，或这是一条对 ASID 定向失效
    /// 免疫的全局条目。两者都不参与迁移。
    #[error("this mapping cannot be withdrawn and confirmed")]
    Pinned,
    #[error("the page carrier has been moved out")]
    Missing,
    #[error("the page slot is held by another use")]
    Busy,
}

/// 物理来源失败折入地址空间失败码：`OutOfRange` 对应用户可见的"没有所属区域"。
/// `Missing` 与 `Busy` 各留自己的身份，不折成别的失败掩盖。
impl From<FlatError> for NestError {
    fn from(error: FlatError) -> Self {
        match error {
            FlatError::OutOfMemory => NestError::OutOfMemory,
            FlatError::NotAligned => NestError::NotAligned,
            FlatError::OutOfRange => NestError::NoRegion,
            FlatError::Missing => NestError::Missing,
            FlatError::Busy => NestError::Busy,
        }
    }
}
