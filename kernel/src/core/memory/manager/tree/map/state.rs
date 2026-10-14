use alloc::sync::Arc;
use alloc::vec::Vec;

use crate::core::memory::PAGE_SIZE;
use crate::core::memory::manager::addr::VirtAddr;
use crate::core::memory::manager::frame::Frame;
use crate::core::memory::manager::MapError;

use super::super::BorrowedRegion;
use super::range::MapRange;
use super::runs::FrameRuns;

pub(in super::super) struct MapState {
    pub(in super::super) ranges: Vec<MapRange>,
    pub(in super::super) frames: Vec<(usize, Arc<Frame>)>,
    pub(in super::super) borrowed: Option<Arc<BorrowedRegion>>,
    pub(in super::super) borrowed_offset: usize,
    pub(in super::super) lazy: bool,
}

impl MapState {
    pub(in super::super) fn new(ranges: Vec<MapRange>) -> Self {
        Self {
            ranges,
            frames: Vec::new(),
            borrowed: None,
            borrowed_offset: 0,
            lazy: false,
        }
    }

    /// 活区间是否连续覆盖 [lo, hi]。
    pub(in super::super) fn covers(&self, lo: usize, hi: usize) -> bool {
        MapRange::cover(&self.ranges, lo, hi)
    }

    /// 覆盖 `address` 的活区间。
    pub(in super::super) fn range_at(&self, address: usize) -> Option<&MapRange> {
        self.ranges.iter().find(|range| range.contains(address))
    }

    /// 该页是否已有自有 Frame。
    pub(in super::super) fn owns_page(&self, page: usize) -> bool {
        self.frames.iter().any(|(owned, _)| *owned == page)
    }

    /// 按页号顺序插入自有 Frame。
    pub(in super::super) fn insert_frame(&mut self, page: usize, frame: Arc<Frame>) {
        let at = self.frames.partition_point(|(owned, _)| *owned < page);
        self.frames.insert(at, (page, frame));
    }

    /// 页窗口 [first, last) 内的自有 Frame，页号重定位到窗口起点。
    pub(in super::super) fn frames_in_pages(
        &self,
        first: usize,
        last: usize,
    ) -> Result<Vec<(usize, Arc<Frame>)>, MapError> {
        let mut frames = Vec::new();
        frames
            .try_reserve(
                self.frames
                    .iter()
                    .filter(|(page, _)| *page >= first && *page < last)
                    .count(),
            )
            .map_err(|_| MapError::OutOfMemory)?;
        frames.extend(
            self.frames
                .iter()
                .filter(|(page, _)| *page >= first && *page < last)
                .map(|(page, frame)| (*page - first, frame.clone())),
        );
        Ok(frames)
    }

    /// 已发布 PTE 的页数：自有 Frame 加借用的整段区间。
    pub(in super::super) fn pte_count(&self) -> usize {
        self.frames.len()
            + self.borrowed.as_ref().map_or(0, |_| {
                self.ranges
                    .iter()
                    .map(|range| range.size.get() / PAGE_SIZE)
                    .sum()
            })
    }

    /// 帧分段：以 `base` 为虚拟起点、按区间 current 与物理连续性切段。
    pub(in super::super) fn runs<'a>(
        &'a self,
        base: VirtAddr,
        frames: &'a [(usize, Arc<Frame>)],
    ) -> FrameRuns<'a> {
        FrameRuns::new(base, frames, &self.ranges)
    }
}
