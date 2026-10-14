use env::MemoryFail;
use fack::prelude::Error;

#[derive(Error, Debug, Clone, Copy, PartialEq, Eq)]
pub enum MapError {
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

/// 页表失败折成线上码：`Denied` 兼收 `Pinned`、`NotMapped` 与 `DramOverlap`。
impl From<MapError> for MemoryFail {
    fn from(error: MapError) -> Self {
        match error {
            MapError::OutOfMemory => MemoryFail::OoM,
            MapError::NotAligned => MemoryFail::NotAligned,
            MapError::NoRegion => MemoryFail::NoRegion,
            MapError::AlreadyMapped => MemoryFail::AlreadyMapped,
            MapError::WidenDenied => MemoryFail::WidenDenied,
            MapError::Shootdown => MemoryFail::Busy,
            MapError::Pinned => MemoryFail::Denied,
            MapError::NotMapped | MapError::DramOverlap => MemoryFail::Denied,
        }
    }
}
