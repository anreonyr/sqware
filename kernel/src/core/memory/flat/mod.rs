//! 物理来源：分配器 RAM 的逻辑页持有，或外部借用物理区间的凭证。
//!
//! Flat 不含 VA、页表、Task 或授权账目；它是唯一的物化权威。

pub mod allocator;

mod allocation;
mod custody;
mod error;
mod frame;
mod pages;
mod range;
mod region;
mod residency;
mod residency_entry;
mod slot;
mod source;
mod state;
pub(crate) mod transfer;

use alloc::sync::Arc;
use alloc::vec::Vec;

use crate::core::memory::PAGE_SIZE;
use crate::core::memory::address::{PageCount, PageIndex, PageOffset, PhysAddr};
use crate::machine::sync::{Level, SpinLock};

pub use allocation::Allocation;
pub use error::FlatError;
pub use frame::Frame;
pub use pages::Pages;
pub use range::FlatRange;
pub use region::BorrowedRegion;
pub use residency::Residency;
#[cfg(debug_assertions)]
pub(crate) use transfer::fail_transfer_prepare;

use residency_entry::ResidencyEntry;
use slot::Slot;
use source::FlatSource;
use state::FlatState;

/// 线性逻辑页来源。
///
/// 要么是分配器 RAM（页槽按需物化），要么是调用方仍保留的外部物理区间。
/// 物化时机由调用方组合决定，来源本身不携带 eager / lazy 策略。
pub struct Flat {
    page_count: PageCount,
    /// 由 `from_frames` 导入建立的来源：不是普通 `acquire` 来源，不能参与迁移。
    imported: bool,
    state: SpinLock<FlatState>,
}

impl Flat {
    /// 建立 RAM 逻辑来源；不提前物化。
    pub fn acquire(allocation: Allocation) -> Result<Arc<Self>, FlatError> {
        Self::new(
            allocation.page_count(),
            FlatSource::Managed(Vec::new()),
            false,
        )
    }

    /// 可信内部接入合法既有帧：不清零、不改变 PA 或内容。
    ///
    /// 输入必须按页号严格递增；越界、重复与乱序都是 `OutOfRange`。
    /// 接入的帧同时被登记为“已在别处”：它们不再参与迁移。
    pub fn from_frames(
        page_count: PageCount,
        frames: &[(PageIndex, Arc<Frame>)],
    ) -> Result<Arc<Self>, FlatError> {
        let mut slots = Vec::new();
        slots
            .try_reserve_exact(frames.len())
            .map_err(|_| FlatError::OutOfMemory)?;
        let mut previous = None;
        for (index, frame) in frames {
            let page = index.get();
            if page >= page_count.get() || previous.is_some_and(|previous| previous >= page) {
                return Err(FlatError::OutOfRange);
            }
            previous = Some(page);
            slots.push((*index, Slot::Resident(frame.clone())));
        }
        let flat = Self::new(page_count, FlatSource::Managed(slots), true)?;
        // 可失败的构造都过了，最后才登记托管：导入是永久效果，重复登记是幂等的。
        custody::register(frames.iter().map(|(_, frame)| frame.custody()))?;
        Ok(flat)
    }

    /// 保留外部借用物理区间：不分配数据页、不清零、不按 RAM 回收。
    pub fn from_region(region: Arc<BorrowedRegion>) -> Result<Arc<Self>, FlatError> {
        let page_count = PageCount::new(region.size() / PAGE_SIZE).ok_or(FlatError::OutOfRange)?;
        Self::new(page_count, FlatSource::Borrowed(region), false)
    }

    fn new(
        page_count: PageCount,
        source: FlatSource,
        imported: bool,
    ) -> Result<Arc<Self>, FlatError> {
        Arc::try_new(Self {
            page_count,
            imported,
            state: SpinLock::new_level(Level::Flat, FlatState::new(source)),
        })
        .map_err(|_| FlatError::OutOfMemory)
    }

    /// 来源声明的逻辑页总数。
    pub fn page_count(&self) -> PageCount {
        self.page_count
    }

    /// 整段来源。
    pub fn whole(self: &Arc<Self>) -> FlatRange {
        FlatRange::new(self.clone(), PageIndex::ZERO, self.page_count())
    }

    /// 按来源位置选择范围。
    pub fn range(
        self: &Arc<Self>,
        index: PageIndex,
        count: PageCount,
    ) -> Result<FlatRange, FlatError> {
        let end = index
            .get()
            .checked_add(count.get())
            .ok_or(FlatError::OutOfRange)?;
        if end > self.page_count.get() {
            return Err(FlatError::OutOfRange);
        }
        Ok(FlatRange::new(self.clone(), index, count))
    }

    pub(super) fn is_borrowed(&self) -> bool {
        matches!(&self.state.lock().source, FlatSource::Borrowed(_))
    }

    pub(super) fn reside(&self, range: FlatRange) -> Result<Residency, FlatError> {
        let mut state = self.state.lock();
        let next = state
            .next_residency
            .checked_add(1)
            .ok_or(FlatError::OutOfMemory)?;
        state
            .residencies
            .try_reserve(1)
            .map_err(|_| FlatError::OutOfMemory)?;
        let id = state.next_residency;
        state.residencies.push(ResidencyEntry {
            id,
            index: range.index(),
            count: range.page_count(),
        });
        state.next_residency = next;
        Ok(Residency::new(range, id))
    }

    pub(super) fn release_residency(&self, id: usize) {
        let mut state = self.state.lock();
        let at = state
            .residencies
            .iter()
            .position(|entry| entry.id == id)
            .expect("residency belongs to its source");
        state.residencies.remove(at);
    }

    pub(super) fn is_pinned(&self, index: PageIndex, count: PageCount) -> bool {
        self.state.lock().is_pinned(index, count)
    }

    /// 物化 [index, index + count)：只有 Fresh 的页会被分配，已有承载的页跳过。
    ///
    /// 已是 `Missing` 的页不补零也不新分配——那是承载移走后的墓碑，
    /// 只有移交能把新承载交进它。中途失败保留已经成功的页槽；调用方仍可重试。
    pub(super) fn materialize_range(
        &self,
        index: PageIndex,
        count: PageCount,
    ) -> Result<(), FlatError> {
        let end = index
            .get()
            .checked_add(count.get())
            .ok_or(FlatError::OutOfRange)?;
        if end > self.page_count.get() {
            return Err(FlatError::OutOfRange);
        }
        let mut guard = self.state.lock();
        let FlatSource::Managed(slots) = &mut guard.source else {
            return Ok(());
        };
        let mut page = index.get();
        let mut at = slots.partition_point(|(slot, _)| slot.get() < page);
        while page < end {
            if slots.get(at).is_some_and(|(slot, _)| slot.get() == page) {
                if matches!(slots[at].1, Slot::Missing) {
                    return Err(FlatError::Missing);
                }
                page += 1;
                at += 1;
                continue;
            }
            slots.try_reserve(1).map_err(|_| FlatError::OutOfMemory)?;
            let frame = Frame::allocate()?;
            slots.insert(at, (PageIndex::new(page), Slot::Resident(frame)));
            page += 1;
            at += 1;
        }
        Ok(())
    }

    /// 窗口内已物化页槽的稀疏快照，偏移相对窗口起点。
    ///
    /// `Missing` 是墓碑，不是承载：它不进快照。
    pub(super) fn materialized(
        &self,
        index: PageIndex,
        count: PageCount,
    ) -> Result<Vec<(PageOffset, Arc<Frame>)>, FlatError> {
        let end = index
            .get()
            .checked_add(count.get())
            .ok_or(FlatError::OutOfRange)?;
        if end > self.page_count.get() {
            return Err(FlatError::OutOfRange);
        }
        let guard = self.state.lock();
        let FlatSource::Managed(slots) = &guard.source else {
            return Ok(Vec::new());
        };
        let first = slots.partition_point(|(slot, _)| slot.get() < index.get());
        let last = slots.partition_point(|(slot, _)| slot.get() < end);
        let mut out = Vec::new();
        out.try_reserve(last - first)
            .map_err(|_| FlatError::OutOfMemory)?;
        for (slot, state) in &slots[first..last] {
            if let Slot::Resident(frame) = state {
                out.push((PageOffset::new(slot.get() - index.get()), frame.clone()));
            }
        }
        Ok(out)
    }

    /// 该窗口是否每一页都是 Managed 的承载；借用来源没有物化页槽，不算。
    ///
    /// 只读快照：不物化、不分配。`Missing` 与 Fresh 一样不算承载。
    pub(super) fn fully_materialized(
        &self,
        index: PageIndex,
        count: PageCount,
    ) -> Result<bool, FlatError> {
        let end = index
            .get()
            .checked_add(count.get())
            .ok_or(FlatError::OutOfRange)?;
        if end > self.page_count.get() {
            return Err(FlatError::OutOfRange);
        }
        let guard = self.state.lock();
        let FlatSource::Managed(slots) = &guard.source else {
            return Ok(false);
        };
        let first = slots.partition_point(|(slot, _)| slot.get() < index.get());
        let last = slots.partition_point(|(slot, _)| slot.get() < end);
        let mut expected = index.get();
        for (slot, state) in &slots[first..last] {
            if slot.get() == expected && matches!(state, Slot::Resident(_)) {
                expected += 1;
            }
        }
        Ok(expected == end)
    }

    pub(super) fn physical_at(&self, page: PageIndex) -> Result<PhysAddr, FlatError> {
        if page.get() >= self.page_count.get() {
            return Err(FlatError::OutOfRange);
        }
        let guard = self.state.lock();
        match &guard.source {
            FlatSource::Borrowed(region) => {
                let offset = page
                    .get()
                    .checked_mul(PAGE_SIZE)
                    .ok_or(FlatError::OutOfRange)?;
                let address = region
                    .start()
                    .as_usize()
                    .checked_add(offset)
                    .ok_or(FlatError::OutOfRange)?;
                Ok(PhysAddr::from_raw(address))
            }
            FlatSource::Managed(slots) => {
                let (at, present) = slot::spot(slots, page);
                if !present {
                    return Err(FlatError::OutOfRange);
                }
                match &slots[at].1 {
                    Slot::Resident(frame) => Ok(PhysAddr::from_raw(frame.physical_address())),
                    // 承载已移出：这是页槽自己的状态，不抹成“越界”。
                    Slot::Missing => Err(FlatError::Missing),
                }
            }
        }
    }
}
