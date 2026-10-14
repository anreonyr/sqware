mod access;
mod active;
mod range;
mod run;
mod runs;
mod state;

use alloc::sync::Arc;
use alloc::vec::Vec;

use crate::core::memory::PAGE_SIZE;
use crate::core::memory::address::{PageCount, PageOffset, VirtAddr};
use crate::core::memory::flat::FlatRange;
#[cfg(debug_assertions)]
use crate::core::memory::flat::Frame;
use crate::core::memory::nest::NestError;
use crate::core::memory::nest::reservation::Reservation;
use crate::core::memory::nest::table::PteFlags;
use crate::machine::sync::{Level, SpinLock};

pub use access::Access;
pub use active::ActiveRange;
pub(crate) use range::ViewRange;

use state::ViewState;

/// 一段虚拟区间的关系：真实占用、强持的平来源与活动库存。
///
/// U/G/A/D 是整段关系共有的架构属性，随挂载政策定下；改权不动它。
pub struct View {
    pub(in crate::core::memory::nest) reservation: Reservation,
    pub(in crate::core::memory::nest) attrs: PteFlags,
    pub(in crate::core::memory::nest) state: SpinLock<ViewState>,
}

impl core::fmt::Debug for View {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("View")
            .field("initial_start", &self.initial_start())
            .field("initial_size", &self.initial_size())
            .finish_non_exhaustive()
    }
}

impl View {
    /// 建立一段关系；活动区间必须落在初始范围内，来源跨度必须等长。
    pub(in crate::core::memory::nest) fn new(
        reservation: Reservation,
        attrs: PteFlags,
        active: Vec<ActiveRange>,
        source: FlatRange,
    ) -> Result<Self, NestError> {
        let (start, size) = (reservation.start(), reservation.size());
        let end = start
            .as_usize()
            .checked_add(size - 1)
            .ok_or(NestError::NotAligned)?;
        if size == 0
            || !size.is_multiple_of(PAGE_SIZE)
            || active.is_empty()
            || source.page_count().checked_bytes() != Some(size)
        {
            return Err(NestError::NotAligned);
        }
        for range in &active {
            let last = range.end();
            if range.start.as_usize() < start.as_usize() || last > end {
                return Err(NestError::NoRegion);
            }
        }
        if active
            .windows(2)
            .any(|pair| pair[0].end() >= pair[1].start.as_usize())
        {
            return Err(NestError::AlreadyMapped);
        }
        Ok(Self {
            reservation,
            attrs,
            state: SpinLock::new_level(Level::View, ViewState::new(active, source)),
        })
    }

    pub fn initial_start(&self) -> VirtAddr {
        self.reservation.start()
    }

    pub fn initial_size(&self) -> usize {
        self.reservation.size()
    }

    pub fn initial_range(&self) -> (VirtAddr, usize) {
        (self.initial_start(), self.initial_size())
    }

    /// 认住它的那个 Nest；已关闭的 Nest 不算。
    pub fn owner(&self) -> Option<Arc<super::Nest>> {
        let nest = self.reservation.nest()?;
        (!nest.is_closed()).then_some(nest)
    }

    /// 活动库存的快照。
    pub fn active(&self) -> Result<Vec<ActiveRange>, NestError> {
        let state = self.state.lock();
        let mut active = Vec::new();
        active
            .try_reserve(state.active.len())
            .map_err(|_| NestError::OutOfMemory)?;
        active.extend_from_slice(&state.active);
        Ok(active)
    }

    /// 已发布 PTE 的页数。
    pub fn frame_count(&self) -> usize {
        self.state.lock().published.len()
    }

    pub fn is_live(&self) -> bool {
        self.owner().is_some() && !self.state.lock().active.is_empty()
    }

    /// 整段选择：操作时按当前活动与未结算退休解析。
    pub fn whole(self: &Arc<Self>) -> ViewRange {
        ViewRange::Whole(self.clone())
    }

    /// 初始范围内的固定页坐标选择；越界即拒。
    pub fn range(
        self: &Arc<Self>,
        offset: PageOffset,
        count: PageCount,
    ) -> Result<ViewRange, NestError> {
        ViewRange::pages(self, offset, count)
    }

    pub(in crate::core::memory::nest) fn source(&self) -> FlatRange {
        self.state.lock().source.clone()
    }

    /// 一段当前权限对应的叶子 PTE 位：V、RWX 与整段关系的 U/G/A/D。
    pub(in crate::core::memory::nest) fn leaf_flags(&self, current: Access) -> PteFlags {
        PteFlags::V | current.to_pte() | self.attrs
    }

    /// 架构属性只含 U/G/A/D；给出其它位就拒。
    pub(in crate::core::memory::nest) fn attrs_are_architectural(attrs: PteFlags) -> bool {
        !attrs.intersects(PteFlags::V | PteFlags::R | PteFlags::W | PteFlags::X)
    }

    #[cfg(debug_assertions)]
    pub fn frames_in(
        &self,
        start: VirtAddr,
        size: usize,
    ) -> Result<Vec<(usize, Arc<Frame>)>, NestError> {
        if size == 0 || start.offset() != 0 || size & (PAGE_SIZE - 1) != 0 {
            return Err(NestError::NotAligned);
        }
        let end = start
            .as_usize()
            .checked_add(size - 1)
            .ok_or(NestError::NotAligned)?;
        let state = self.state.lock();
        if !state.covers(start.as_usize(), end) || state.source.is_borrowed() {
            return Err(NestError::NoRegion);
        }
        let first = start
            .as_usize()
            .checked_sub(self.initial_start().as_usize())
            .ok_or(NestError::NoRegion)?
            / PAGE_SIZE;
        let pages = size / PAGE_SIZE;
        let last = first.checked_add(pages).ok_or(NestError::NotAligned)?;
        let count = state
            .published
            .iter()
            .filter(|(page, _)| *page >= first && *page < last)
            .count();
        if count != pages {
            return Err(NestError::NoRegion);
        }
        let mut out = Vec::new();
        out.try_reserve_exact(pages)
            .map_err(|_| NestError::OutOfMemory)?;
        out.extend(
            state
                .published
                .iter()
                .filter(|(page, _)| *page >= first && *page < last)
                .map(|(page, frame)| (page - first, frame.clone())),
        );
        Ok(out)
    }

    /// `address` 相对初始范围的页号；调用方保证它在初始范围内。
    /// 活动区间的只读快照（debug 审计用：跨关系比较不能套着关系锁）。
    #[cfg(debug_assertions)]
    pub(in crate::core::memory::nest) fn active_snapshot(&self) -> Vec<(usize, usize)> {
        self.state
            .lock()
            .active
            .iter()
            .map(|range| (range.start.as_usize(), range.end()))
            .collect()
    }

    pub(in crate::core::memory::nest) fn page_index(&self, address: VirtAddr) -> usize {
        (address.as_usize() - self.initial_start().as_usize()) / PAGE_SIZE
    }

    /// 活动区间是否与 [lo, hi] 相交。
    pub(in crate::core::memory::nest) fn overlaps(&self, lo: usize, hi: usize) -> bool {
        self.state
            .lock()
            .active
            .iter()
            .any(|range| range.overlaps(lo, hi))
    }
}
