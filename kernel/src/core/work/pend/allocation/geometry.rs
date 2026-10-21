//! 一段**帧无关的页窗口**：相对某份来源起点的坐标。
//!
//! 它给两个用途共用：派生子段的几何（父侧分配槽与子记录共享同一个 `Arc`），
//! 以及独立持有在来源记录上的登记窗口。它不持 `FlatRange`、`Frame` 或 `PoleMeta`，
//! 所以记住一段窗口不会强持来源；换成本地绝对页号只经 `FlatRange::range` 一次。

use alloc::sync::Arc;

use crate::core::memory::address::{PageCount, PageOffset};
use env::PieFail;

pub(crate) struct Geometry {
    offset: PageOffset,
    count: PageCount,
}

impl Geometry {
    /// 页窗口只在这一处成型，之后没有放宽的路。
    pub(crate) fn new(offset: PageOffset, count: PageCount) -> Result<Arc<Self>, PieFail> {
        Arc::try_new(Self { offset, count }).map_err(|_| PieFail::OoM)
    }

    /// 这一段窗口（相对某份来源起点的页偏移与页数）。
    pub(crate) fn window(&self) -> (PageOffset, PageCount) {
        (self.offset, self.count)
    }

    /// 两段页窗口是否有交集；相加溢出按相交算（保守），不裁剪任何一侧。
    pub(crate) fn overlaps(&self, offset: PageOffset, count: PageCount) -> bool {
        let start = self.offset.get();
        let end = start.saturating_add(self.count.get());
        let other = offset.get();
        let other_end = other.saturating_add(count.get());
        start < other_end && other < end
    }
}
