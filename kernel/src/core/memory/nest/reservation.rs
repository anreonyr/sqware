use alloc::sync::{Arc, Weak};

use crate::core::memory::address::VirtAddr;
use crate::core::memory::nest::Nest;
use crate::core::memory::nest::address::RegionId;

/// 一笔地址保留：只认住它的那个地址空间（`Weak`，升级失败即失效）。
///
/// 它不参与内存回收的所有权判定；占用表里的 generation 才是回收凭证。
pub struct Reservation {
    nest: Weak<Nest>,
    region: RegionId,
    start: VirtAddr,
    size: usize,
    id: usize,
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
    pub(in crate::core::memory::nest) fn new(
        nest: Weak<Nest>,
        region: RegionId,
        start: VirtAddr,
        size: usize,
        id: usize,
    ) -> Self {
        Self {
            nest,
            region,
            start,
            size,
            id,
        }
    }

    pub fn start(&self) -> VirtAddr {
        self.start
    }

    pub fn size(&self) -> usize {
        self.size
    }

    pub fn region(&self) -> RegionId {
        self.region
    }

    /// 占用的 generation。
    pub(in crate::core::memory::nest) fn id(&self) -> usize {
        self.id
    }

    /// 含尾最后一个字节。
    pub(in crate::core::memory::nest) fn last(&self) -> usize {
        self.start.as_usize() + (self.size - 1)
    }

    pub(in crate::core::memory::nest) fn nest(&self) -> Option<Arc<Nest>> {
        self.nest.upgrade()
    }
}

unsafe impl Send for Reservation {}
unsafe impl Sync for Reservation {}
