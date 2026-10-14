use crate::core::memory::PAGE_SIZE;
use crate::core::memory::address::PageCount;

use super::error::FlatError;

/// 来源的物理分配约束：逻辑页数与每页起址的对齐下界。
///
/// 对齐只约束每个页地址的下界，不表示各页物理连续；不支持的约束在构造时明确拒绝，
/// 不接受后忽略。PR1 只接受页大小对齐。
pub struct Allocation {
    pages: PageCount,
}

impl Allocation {
    pub fn new(pages: usize, align: usize) -> Result<Self, FlatError> {
        let pages = PageCount::new(pages).ok_or(FlatError::NotAligned)?;
        if align != PAGE_SIZE {
            return Err(FlatError::NotAligned);
        }
        Ok(Self { pages })
    }

    pub fn page_count(&self) -> PageCount {
        self.pages
    }
}
