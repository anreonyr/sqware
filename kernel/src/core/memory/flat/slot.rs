use alloc::sync::Arc;

use crate::core::memory::address::PageIndex;

use super::frame::Frame;

/// 一个逻辑页槽的状态；页槽表里缺席即 Fresh（从未物化）。
///
/// `Resident` 是唯一拥有承载的空闲态；`Missing` 是承载已移走的墓碑，
/// 不得再自动补齐（只有移交能把新承载交进它）。
/// `Moving` 是“一次移交已取走这一页的承载”：事务身份加上帧本身，
/// 于是即使中途 abort，这一页也不会失去它的承载（fail-closed）。
/// `Reserved` 是“一次移交给这一页留了空位”，尚未交给承载。
/// 两者都不算空闲态：物化、PA 出口与发布都与它们互斥。
pub(super) enum Slot {
    Resident(Arc<Frame>),
    Missing,
    Moving {
        frame: Arc<Frame>,
        transaction: usize,
    },
    /// 一次移交给这一页留的空位；记着它原本是 Fresh 还是 `Missing`，
    /// 于是事务中途回滚能把这一页原样还回去。
    Reserved {
        transaction: usize,
        was_missing: bool,
    },
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
