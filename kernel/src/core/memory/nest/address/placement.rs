use crate::core::memory::PAGE_SIZE;
use crate::core::memory::address::VirtAddr;
use crate::core::memory::nest::NestError;
use crate::core::memory::nest::address::{PlacementAt, RegionId};

/// 一次地址选择：选址窗口、跨度、起始对齐与选址方式。
/// 跨度非零且按页；`Exact` 的起点必须按页且满足对齐。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Placement {
    region: RegionId,
    size: usize,
    align: usize,
    at: PlacementAt,
}

impl Placement {
    pub(crate) fn new(
        region: RegionId,
        size: usize,
        align: usize,
        at: PlacementAt,
    ) -> Result<Self, NestError> {
        if size == 0 || !size.is_multiple_of(PAGE_SIZE) || !align.is_power_of_two() {
            return Err(NestError::NotAligned);
        }
        if let PlacementAt::Exact(start) = at
            && (start.offset() != 0 || start.as_usize() % align != 0)
        {
            return Err(NestError::NotAligned);
        }
        Ok(Self {
            region,
            size,
            align,
            at,
        })
    }

    /// 窗口内最低的满足 `align` 的空档。
    pub(crate) fn lowest(region: RegionId, size: usize, align: usize) -> Result<Self, NestError> {
        Self::new(region, size, align, PlacementAt::Lowest)
    }

    /// 窗口内最高的空档。
    pub(crate) fn highest(region: RegionId, size: usize) -> Result<Self, NestError> {
        Self::new(region, size, PAGE_SIZE, PlacementAt::Highest)
    }

    /// 固定跨度：调用方已经知道地址。
    pub(crate) fn exact(region: RegionId, start: VirtAddr, size: usize) -> Result<Self, NestError> {
        Self::new(region, size, PAGE_SIZE, PlacementAt::Exact(start))
    }

    pub(crate) fn region(self) -> RegionId {
        self.region
    }

    pub(crate) fn size(self) -> usize {
        self.size
    }

    pub(crate) fn align(self) -> usize {
        self.align
    }

    pub(crate) fn at(self) -> PlacementAt {
        self.at
    }
}
