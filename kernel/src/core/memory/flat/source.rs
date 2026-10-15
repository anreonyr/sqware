use alloc::sync::Arc;
use alloc::vec::Vec;

use crate::core::memory::address::PageIndex;

use super::region::BorrowedRegion;
use super::slot::Slot;

/// Flat 的物理来源。两种来源共用范围选择与物化协议，回收规则不同：
/// Managed 的页由分配器回收，Borrowed 的存储不归 RAM 分配器。
pub(super) enum FlatSource {
    /// 逻辑页槽，按页号严格递增；缺席的页是 Fresh（不占槽）。
    Managed(Vec<(PageIndex, Slot)>),
    /// 外部保证存活的物理区间；不分配其数据页、不清零、不按 RAM 回收。
    Borrowed(Arc<BorrowedRegion>),
}
