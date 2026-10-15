use alloc::sync::Arc;

use crate::core::memory::address::PageIndex;

use super::frame::Frame;

/// 一个逻辑页槽的状态；页槽表里缺席即 Fresh（从未物化）。
///
/// `Resident` 是唯一拥有承载的状态；`Missing` 是承载已移走的墓碑，
/// 不得再自动补齐——只有移交可以把新承载交进它。
pub(super) enum Slot {
    Resident(Arc<Frame>),
    Missing,
}

/// 页槽在按页号严格递增的表里的位置，以及这一页是不是已经占了槽。
pub(super) fn spot(slots: &[(PageIndex, Slot)], page: PageIndex) -> (usize, bool) {
    let at = slots.partition_point(|(slot, _)| slot.get() < page.get());
    (
        at,
        slots
            .get(at)
            .is_some_and(|(slot, _)| slot.get() == page.get()),
    )
}
