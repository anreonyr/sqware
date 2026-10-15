use crate::core::memory::address::{PageCount, PageOffset, PhysAddr};

use super::error::FlatError;
use super::residency::Residency;

/// 一个来源范围的完整、稳定物化结果。
///
/// Managed 的页此时已全部物化，Borrowed 的借用凭证仍被引用持有；它只由
/// `FlatRange::materialize` 构造，不用裸 `Vec<PhysAddr>` 代替持有。
pub struct Pages {
    residency: Residency,
}

impl Pages {
    pub(super) fn new(residency: Residency) -> Self {
        Self { residency }
    }

    /// 该范围的页数。
    #[allow(dead_code)]
    pub fn page_count(&self) -> PageCount {
        self.residency.range().page_count()
    }

    /// 范围起点之内第 `offset` 页的物理地址；物理地址是地址信息，不是持有凭证。
    pub(crate) fn physical(&self, offset: PageOffset) -> Result<PhysAddr, FlatError> {
        self.residency.range().physical(offset)
    }
}
