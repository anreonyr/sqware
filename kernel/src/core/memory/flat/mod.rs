//! 物理来源：分配器 RAM 的逻辑页持有，或外部借用物理区间的凭证。
//!
//! Flat 不含 VA、页表、Task 或授权账目；它是唯一的物化权威。

pub mod allocator;

mod allocation;
mod error;
mod frame;
mod pages;
mod range;
mod region;
mod source;

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

use source::FlatSource;

/// 线性逻辑页来源。
///
/// 要么是分配器 RAM（页槽按需物化），要么是调用方仍保留的外部物理区间。
/// 物化时机由调用方组合决定，来源本身不携带 eager / lazy 策略。
pub struct Flat {
    page_count: PageCount,
    source: SpinLock<FlatSource>,
}

impl Flat {
    /// 建立 RAM 逻辑来源；不提前物化。
    pub fn acquire(allocation: Allocation) -> Result<Arc<Self>, FlatError> {
        Self::new(allocation.page_count(), FlatSource::Managed(Vec::new()))
    }

    /// 可信内部接入合法既有帧：不清零、不改变 PA 或内容。
    ///
    /// 输入必须按页号严格递增；越界、重复与乱序都是 `OutOfRange`。
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
            slots.push((*index, frame.clone()));
        }
        Self::new(page_count, FlatSource::Managed(slots))
    }

    /// 保留外部借用物理区间：不分配数据页、不清零、不按 RAM 回收。
    pub fn from_region(region: Arc<BorrowedRegion>) -> Result<Arc<Self>, FlatError> {
        let page_count = PageCount::new(region.size() / PAGE_SIZE).ok_or(FlatError::OutOfRange)?;
        Self::new(page_count, FlatSource::Borrowed(region))
    }

    fn new(page_count: PageCount, source: FlatSource) -> Result<Arc<Self>, FlatError> {
        Arc::try_new(Self {
            page_count,
            source: SpinLock::new_level(Level::Flat, source),
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
        matches!(&*self.source.lock(), FlatSource::Borrowed(_))
    }

    /// 物化 [index, index + count)：同一逻辑页只有一个结果，槽只增不减。
    ///
    /// 中途失败保留已经成功的页槽；调用方仍可重试。
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
        let mut guard = self.source.lock();
        let FlatSource::Managed(slots) = &mut *guard else {
            return Ok(());
        };
        let mut page = index.get();
        let mut at = slots.partition_point(|(slot, _)| slot.get() < page);
        while page < end {
            if slots.get(at).is_some_and(|(slot, _)| slot.get() == page) {
                page += 1;
                at += 1;
                continue;
            }
            slots.try_reserve(1).map_err(|_| FlatError::OutOfMemory)?;
            let frame = Frame::allocate()?;
            slots.insert(at, (PageIndex::new(page), frame));
            page += 1;
            at += 1;
        }
        Ok(())
    }

    /// 窗口内已物化页槽的稀疏快照，偏移相对窗口起点。
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
        let guard = self.source.lock();
        let FlatSource::Managed(slots) = &*guard else {
            return Ok(Vec::new());
        };
        let first = slots.partition_point(|(slot, _)| slot.get() < index.get());
        let last = slots.partition_point(|(slot, _)| slot.get() < end);
        let mut out = Vec::new();
        out.try_reserve(last - first)
            .map_err(|_| FlatError::OutOfMemory)?;
        for (slot, frame) in &slots[first..last] {
            out.push((PageOffset::new(slot.get() - index.get()), frame.clone()));
        }
        Ok(out)
    }

    /// 该窗口是否已是 Managed 的完整物化结果；借用来源没有物化页槽，不算。
    ///
    /// 只读快照：不物化、不分配。页槽按页号严格递增且唯一，
    /// 因此首尾两页都在窗口内就说明整窗连续覆盖。
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
        let guard = self.source.lock();
        let FlatSource::Managed(slots) = &*guard else {
            return Ok(false);
        };
        let first = slots.partition_point(|(slot, _)| slot.get() < index.get());
        if slots.len() - first < count.get() {
            return Ok(false);
        }
        Ok(
            slots[first].0.get() == index.get()
                && slots[first + count.get() - 1].0.get() == end - 1,
        )
    }

    pub(super) fn physical_at(&self, page: PageIndex) -> Result<PhysAddr, FlatError> {
        if page.get() >= self.page_count.get() {
            return Err(FlatError::OutOfRange);
        }
        let guard = self.source.lock();
        match &*guard {
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
                let at = slots.partition_point(|(slot, _)| slot.get() < page.get());
                slots
                    .get(at)
                    .filter(|(slot, _)| slot.get() == page.get())
                    .map(|(_, frame)| PhysAddr::from_raw(frame.physical_address()))
                    .ok_or(FlatError::OutOfRange)
            }
        }
    }
}
