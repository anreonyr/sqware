use alloc::sync::Arc;
use alloc::vec::Vec;

use crate::core::memory::address::PageIndex;

use super::frame::Frame;
use super::region::BorrowedRegion;

/// Flat 的物理来源。两种来源共用范围选择与物化协议，回收规则不同：
/// Managed 的页由分配器回收，Borrowed 的存储不归 RAM 分配器。
pub(super) enum FlatSource {
    /// 已物化的逻辑页槽，按页号严格递增；未物化的页不占槽。
    Managed(Vec<(PageIndex, Arc<Frame>)>),
    /// 外部保证存活的物理区间；不分配其数据页、不清零、不按 RAM 回收。
    Borrowed(Arc<BorrowedRegion>),
}
