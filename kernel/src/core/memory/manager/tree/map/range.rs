use alloc::vec::Vec;
use core::num::NonZeroUsize;

use crate::core::memory::manager::addr::VirtAddr;
use crate::core::memory::manager::entry::PteFlags;
use crate::core::memory::manager::MapError;

/// 一段存活虚拟区间：ceiling 是权限上限，current 是当前叶子权限位。
#[derive(Debug, Clone, Copy)]
pub struct MapRange {
    pub start: VirtAddr,
    pub size: NonZeroUsize,
    pub ceiling: PteFlags,
    pub current: PteFlags,
}

impl MapRange {
    /// 含尾区间终点（饱和，不溢出）。
    pub fn end(self) -> usize {
        self.start.as_usize().saturating_add(self.size.get() - 1)
    }

    /// 是否含 `address`。
    pub(in super::super) fn contains(self, address: usize) -> bool {
        self.start.as_usize() <= address && address <= self.end()
    }

    /// 是否与 [lo, hi] 相交。
    pub(in super::super) fn overlaps(self, lo: usize, hi: usize) -> bool {
        self.start.as_usize() <= hi && self.end() >= lo
    }

    /// 与 [lo, hi] 的含尾交集；无交集返回 None。
    pub(in super::super) fn intersection(self, lo: usize, hi: usize) -> Option<(usize, usize)> {
        let lo = self.start.as_usize().max(lo);
        let hi = self.end().min(hi);
        (lo <= hi).then_some((lo, hi))
    }

    /// ranges 是否连续覆盖 [lo, hi]。
    pub(super) fn cover(ranges: &[Self], lo: usize, hi: usize) -> bool {
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

    /// ranges 中覆盖 `address` 的区间当前权限位。
    pub(super) fn current_at(ranges: &[Self], address: usize) -> Option<PteFlags> {
        ranges
            .iter()
            .find(|range| range.contains(address))
            .map(|range| range.current)
    }

    /// 去掉 [lo, hi]（protect = None），或把交叠部分的 RWX 换成 flags。
    pub(in super::super) fn split(
        ranges: &[Self],
        lo: usize,
        hi: usize,
        protect: Option<PteFlags>,
    ) -> Result<Vec<Self>, MapError> {
        let mut out = Vec::new();
        out.try_reserve(ranges.len().saturating_add(2))
            .map_err(|_| MapError::OutOfMemory)?;
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
            if let Some(flags) = protect {
                let part_lo = rlo.max(lo);
                let part_hi = rhi.min(hi);
                out.push(Self {
                    start: VirtAddr::from_raw(part_lo),
                    size: NonZeroUsize::new(part_hi - part_lo + 1).unwrap(),
                    current: (range.current - (PteFlags::R | PteFlags::W | PteFlags::X))
                        | (flags & (PteFlags::R | PteFlags::W | PteFlags::X)),
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

    /// 收窄 [lo, hi]：区间 ceiling 换成 ceiling，RWX 收窄为其与 ceiling 的交集。
    /// 任一交叠区间的 ceiling 不容纳新 ceiling、或结果不是合法叶子权限 ⇒ WidenDenied。
    pub(in super::super) fn narrow(
        ranges: &[Self],
        lo: usize,
        hi: usize,
        ceiling: PteFlags,
    ) -> Result<Vec<Self>, MapError> {
        let rwx = PteFlags::R | PteFlags::W | PteFlags::X;
        let mut next = Vec::new();
        next.try_reserve(ranges.len().saturating_add(2))
            .map_err(|_| MapError::OutOfMemory)?;
        for range in ranges {
            let rlo = range.start.as_usize();
            let rhi = range.end();
            if !range.overlaps(lo, hi) {
                next.push(*range);
                continue;
            }
            if !range.ceiling.contains(ceiling) {
                return Err(MapError::WidenDenied);
            }
            if rlo < lo {
                next.push(Self {
                    size: NonZeroUsize::new(lo - rlo).unwrap(),
                    ..*range
                });
            }
            let mid_lo = rlo.max(lo);
            let mid_hi = rhi.min(hi);
            let current = (range.current - rwx) | ((range.current & rwx) & ceiling);
            if !valid_leaf_flags(current) {
                return Err(MapError::WidenDenied);
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

/// 叶子权限位合法性：RWX 非空，且写必须有读。
pub(in super::super) fn valid_leaf_flags(flags: PteFlags) -> bool {
    let rwx = flags & (PteFlags::R | PteFlags::W | PteFlags::X);
    !rwx.is_empty() && (!rwx.contains(PteFlags::W) || rwx.contains(PteFlags::R))
}
