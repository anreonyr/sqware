use alloc::sync::Weak;

use super::va_allocator::RegionId;
use crate::core::memory::manager::addr::VirtAddr;
use crate::core::memory::manager::tree::Tree;

/// 一笔地址保留：只认住它的那棵树（`Weak`，升级失败即失效），
/// 不参与内存回收的所有权判定。
pub struct Reservation {
    pub(super) tree: Weak<Tree>,
    pub(super) region: RegionId,
    pub(super) start: VirtAddr,
    pub(super) size: usize,
    pub(super) id: usize,
}

impl core::fmt::Debug for Reservation {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Reservation")
            .field("region", &self.region)
            .field("start", &self.start)
            .field("size", &self.size)
            .finish()
    }
}

impl Reservation {
    pub fn start(&self) -> VirtAddr {
        self.start
    }
    pub fn size(&self) -> usize {
        self.size
    }
}

unsafe impl Send for Reservation {}
unsafe impl Sync for Reservation {}
