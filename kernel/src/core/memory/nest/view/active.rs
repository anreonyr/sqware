use alloc::vec::Vec;
use core::num::NonZeroUsize;

use crate::core::memory::address::VirtAddr;
use crate::core::memory::nest::NestError;
use crate::core::memory::nest::view::Access;

/// 一段活动关系区间：`ceiling` 是权限上限，`current` 是当前逻辑权限。
#[derive(Debug, Clone, Copy)]
pub struct ActiveRange {
    pub start: VirtAddr,
    pub size: NonZeroUsize,
    pub ceiling: Access,
    pub current: Access,
}

impl ActiveRange {
    pub(crate) fn new(
        start: VirtAddr,
        size: usize,
        ceiling: Access,
        current: Access,
    ) -> Result<Self, NestError> {
        let size = NonZeroUsize::new(size).ok_or(NestError::NotAligned)?;
        if start.offset() != 0
            || !size.get().is_multiple_of(crate::core::memory::PAGE_SIZE)
            || !ceiling.contains(current)
            || !current.is_leaf_set()
        {
            return Err(NestError::NotAligned);
        }
        Ok(Self {
            start,
            size,
            ceiling,
            current,
        })
    }

    /// 含尾区间终点（饱和，不溢出）。
    pub fn end(self) -> usize {
        self.start.as_usize().saturating_add(self.size.get() - 1)
    }

    /// 是否含 `address`。
    pub(in crate::core::memory::nest) fn contains(self, address: usize) -> bool {
        self.start.as_usize() <= address && address <= self.end()
    }

    /// 是否与 [lo, hi] 相交。
    pub(in crate::core::memory::nest) fn overlaps(self, lo: usize, hi: usize) -> bool {
        self.start.as_usize() <= hi && self.end() >= lo
    }

    /// 与 [lo, hi] 的含尾交集；无交集返回 None。
    pub(in crate::core::memory::nest) fn intersection(
        self,
        lo: usize,
        hi: usize,
    ) -> Option<(usize, usize)> {
        let lo = self.start.as_usize().max(lo);
        let hi = self.end().min(hi);
        (lo <= hi).then_some((lo, hi))
    }

    /// ranges 是否连续覆盖 [lo, hi]。
    pub(in crate::core::memory::nest) fn cover(ranges: &[Self], lo: usize, hi: usize) -> bool {
        let mut at = lo;
        for range in ranges {
            if range.end() < at {
                continue;
            }
            if range.start.as_usize() > at {
                return false;
            }
            if range.end() >= hi {
                return true;
            }
            at = range.end() + 1;
        }
        false
    }

    /// ranges 中覆盖 `address` 的区间当前权限。
    pub(in crate::core::memory::nest) fn current_at(
        ranges: &[Self],
        address: usize,
    ) -> Option<Access> {
        ranges
            .iter()
            .find(|range| range.contains(address))
            .map(|range| range.current)
    }

    /// 去掉 [lo, hi]（`access = None`），或把交叠部分的当前权限换成 `access`。
    pub(in crate::core::memory::nest) fn split(
        ranges: &[Self],
        lo: usize,
        hi: usize,
        access: Option<Access>,
    ) -> Result<Vec<Self>, NestError> {
        let mut out = Vec::new();
        out.try_reserve(ranges.len().saturating_add(2))
            .map_err(|_| NestError::OutOfMemory)?;
        for range in ranges {
            let rlo = range.start.as_usize();
            let rhi = range.end();
            if !range.overlaps(lo, hi) {
                out.push(*range);
                continue;
            }
            if rlo < lo {
                out.push(Self {
                    start: range.start,
                    size: NonZeroUsize::new(lo - rlo).unwrap(),
                    ..*range
                });
            }
            if let Some(access) = access {
                let part_lo = rlo.max(lo);
                let part_hi = rhi.min(hi);
                out.push(Self {
                    start: VirtAddr::from_raw(part_lo),
                    size: NonZeroUsize::new(part_hi - part_lo + 1).unwrap(),
                    current: access,
                    ..*range
                });
            }
            if rhi > hi {
                let part_lo = hi + 1;
                out.push(Self {
                    start: VirtAddr::from_raw(part_lo),
                    size: NonZeroUsize::new(rhi - part_lo + 1).unwrap(),
                    ..*range
                });
            }
        }
        Ok(out)
    }

    /// 收窄 [lo, hi]：ceiling 换成 `ceiling`，current 收窄为它与 ceiling 的交集。
    /// 交叠区间的原 ceiling 不容纳新 ceiling、或结果不是合法叶子权限 ⇒ WidenDenied。
    pub(in crate::core::memory::nest) fn narrow(
        ranges: &[Self],
        lo: usize,
        hi: usize,
        ceiling: Access,
    ) -> Result<Vec<Self>, NestError> {
        let mut next = Vec::new();
        next.try_reserve(ranges.len().saturating_add(2))
            .map_err(|_| NestError::OutOfMemory)?;
        for range in ranges {
            let rlo = range.start.as_usize();
            let rhi = range.end();
            if !range.overlaps(lo, hi) {
                next.push(*range);
                continue;
            }
            if !range.ceiling.contains(ceiling) {
                return Err(NestError::WidenDenied);
            }
            if rlo < lo {
                next.push(Self {
                    size: NonZeroUsize::new(lo - rlo).unwrap(),
                    ..*range
                });
            }
            let mid_lo = rlo.max(lo);
            let mid_hi = rhi.min(hi);
            let current = range.current & ceiling;
            if !current.is_leaf_set() {
                return Err(NestError::WidenDenied);
            }
            next.push(Self {
                start: VirtAddr::from_raw(mid_lo),
                size: NonZeroUsize::new(mid_hi - mid_lo + 1).unwrap(),
                ceiling,
                current,
                ..*range
            });
            if rhi > hi {
                let right = hi + 1;
                next.push(Self {
                    start: VirtAddr::from_raw(right),
                    size: NonZeroUsize::new(rhi - right + 1).unwrap(),
                    ..*range
                });
            }
        }
        Ok(next)
    }
}
