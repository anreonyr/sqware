use alloc::sync::Arc;
use alloc::vec::Vec;

use crate::core::memory::address::{PageCount, PageIndex, PageOffset, PhysAddr};

use super::Flat;
use super::error::FlatError;
use super::frame::Frame;
use super::pages::Pages;

/// 强持同一个 `Flat` 的校验范围。子范围用相对偏移选择，合成后仍是原来源身份，
/// 不建立父范围链、不复制数据。
#[derive(Clone)]
pub struct FlatRange {
    flat: Arc<Flat>,
    index: PageIndex,
    count: PageCount,
}

impl FlatRange {
    pub(super) fn new(flat: Arc<Flat>, index: PageIndex, count: PageCount) -> Self {
        Self { flat, index, count }
    }

    pub fn index(&self) -> PageIndex {
        self.index
    }

    pub fn page_count(&self) -> PageCount {
        self.count
    }

    /// 是不是**同一份来源上的同一段窗口**（Flat 身份与坐标都一致）。
    ///
    /// 用来判断一段关系能不能代表整份来源——子范围/切片都不能。
    pub(crate) fn same_range(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.flat, &other.flat)
            && self.index == other.index
            && self.count == other.count
    }

    /// 相对偏移选择子范围。
    pub fn range(&self, offset: PageOffset, count: PageCount) -> Result<Self, FlatError> {
        let end = offset
            .get()
            .checked_add(count.get())
            .ok_or(FlatError::OutOfRange)?;
        if end > self.count.get() {
            return Err(FlatError::OutOfRange);
        }
        let index = PageIndex::new(
            self.index()
                .get()
                .checked_add(offset.get())
                .ok_or(FlatError::OutOfRange)?,
        );
        self.flat.range(index, count)
    }

    /// 取得完整、稳定的物理持有：Managed 必要时物化，Borrowed 校验并保留借用区间。
    pub fn materialize(&self) -> Result<Pages, FlatError> {
        self.flat.materialize_range(self.index, self.count)?;
        Ok(Pages::new(self.clone()))
    }

    /// 只读稀疏已物化的 `Frame`；不物化、不保证整个范围完整；借用来源没有 `Frame`。
    /// 偏移相对范围起点。
    pub(crate) fn materialized_frames(&self) -> Result<Vec<(PageOffset, Arc<Frame>)>, FlatError> {
        self.flat.materialized(self.index, self.count)
    }

    /// 该范围是否已是 Managed 的完整物化结果；借用来源不算。只读，不物化。
    pub(crate) fn fully_materialized(&self) -> Result<bool, FlatError> {
        self.flat.fully_materialized(self.index, self.count)
    }

    pub(crate) fn is_borrowed(&self) -> bool {
        self.flat.is_borrowed()
    }

    pub(super) fn physical(&self, offset: PageOffset) -> Result<PhysAddr, FlatError> {
        if offset.get() >= self.count.get() {
            return Err(FlatError::OutOfRange);
        }
        let page = PageIndex::new(
            self.index
                .get()
                .checked_add(offset.get())
                .ok_or(FlatError::OutOfRange)?,
        );
        self.flat.physical_at(page)
    }
}
