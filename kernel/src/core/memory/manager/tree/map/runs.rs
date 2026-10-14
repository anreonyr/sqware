use alloc::sync::Arc;

use crate::core::memory::PAGE_SIZE;
use crate::core::memory::manager::addr::{PhysAddr, VirtAddr};
use crate::core::memory::manager::entry::PteFlags;
use crate::core::memory::manager::frame::Frame;
use crate::core::memory::manager::MapError;

use super::range::MapRange;
use super::run::FrameRun;

/// 帧分段迭代器：把 `frames` 切成一串连续段，不再分配。
///
/// 续段条件同时成立：页号相邻、物理地址相邻、所在区间 current 相同。
/// 段起点虚拟地址为 `base + 帧页号 * PAGE_SIZE`；任一帧不在任何区间内 ⇒ NoRegion。
pub(in super::super) struct FrameRuns<'a> {
    base: VirtAddr,
    frames: &'a [(usize, Arc<Frame>)],
    ranges: &'a [MapRange],
    at: usize,
}

impl<'a> FrameRuns<'a> {
    pub(in super::super) fn new(
        base: VirtAddr,
        frames: &'a [(usize, Arc<Frame>)],
        ranges: &'a [MapRange],
    ) -> Self {
        Self {
            base,
            frames,
            ranges,
            at: 0,
        }
    }

    fn address(&self, page: usize) -> VirtAddr {
        self.base + page * PAGE_SIZE
    }

    /// 第 `at` 帧能否续在 current 的段后面。
    fn joins(&self, at: usize, current: PteFlags) -> bool {
        let previous = &self.frames[at - 1];
        let next = &self.frames[at];
        next.0 == previous.0 + 1
            && next.1.physical_address() == previous.1.physical_address() + PAGE_SIZE
            && MapRange::current_at(self.ranges, self.address(next.0).as_usize())
                .map(|flags| flags.bits())
                == Some(current.bits())
    }
}

impl Iterator for FrameRuns<'_> {
    type Item = Result<FrameRun, MapError>;

    fn next(&mut self) -> Option<Self::Item> {
        let first = self.at;
        if first >= self.frames.len() {
            return None;
        }
        self.at = first + 1;
        let current = match MapRange::current_at(
            self.ranges,
            self.address(self.frames[first].0).as_usize(),
        ) {
            Some(current) => current,
            None => return Some(Err(MapError::NoRegion)),
        };
        let mut last = first + 1;
        while last < self.frames.len() && self.joins(last, current) {
            last += 1;
        }
        self.at = last;
        Some(Ok(FrameRun {
            start: self.address(self.frames[first].0),
            physical: PhysAddr::from_raw(self.frames[first].1.physical_address()),
            size: (last - first) * PAGE_SIZE,
            current,
        }))
    }
}
