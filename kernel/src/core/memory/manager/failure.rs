use env::MemoryFail;

use super::MapError;

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
