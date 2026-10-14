use alloc::sync::Arc;
use alloc::vec::Vec;

use crate::core::memory::flat::{FlatRange, Frame};

use super::active::ActiveRange;
use super::runs::FrameRuns;

/// 一段活动关系的库存：活动区间、强持的平来源、已发布页的强持记录。
pub(in crate::core::memory::nest) struct ViewState {
    pub(in crate::core::memory::nest) active: Vec<ActiveRange>,
    pub(in crate::core::memory::nest) source: FlatRange,
    pub(in crate::core::memory::nest) published: Vec<(usize, Arc<Frame>)>,
}

impl ViewState {
    pub(in crate::core::memory::nest) fn new(active: Vec<ActiveRange>, source: FlatRange) -> Self {
        Self {
            active,
            source,
            published: Vec::new(),
        }
    }

    /// 活动区间是否连续覆盖 [lo, hi]。
    pub(in crate::core::memory::nest) fn covers(&self, lo: usize, hi: usize) -> bool {
        ActiveRange::cover(&self.active, lo, hi)
    }

    /// 覆盖 `address` 的活动区间。
    pub(in crate::core::memory::nest) fn range_at(&self, address: usize) -> Option<&ActiveRange> {
        self.active.iter().find(|range| range.contains(address))
    }

    /// 该页是否已有发布记录。
    pub(in crate::core::memory::nest) fn publishes(&self, page: usize) -> bool {
        self.published.iter().any(|(held, _)| *held == page)
    }

    /// 按页号顺序插入发布记录。
    pub(in crate::core::memory::nest) fn publish(&mut self, page: usize, frame: Arc<Frame>) {
        let at = self.published.partition_point(|(held, _)| *held < page);
        self.published.insert(at, (page, frame));
    }

    /// 帧分段：以 `base` 为虚拟起点、按活动区间 current 与物理连续性切段。
    pub(in crate::core::memory::nest) fn runs<'a>(
        &'a self,
        base: crate::core::memory::address::VirtAddr,
        frames: &'a [(usize, Arc<Frame>)],
    ) -> FrameRuns<'a> {
        FrameRuns::new(base, frames, &self.active)
    }
}
