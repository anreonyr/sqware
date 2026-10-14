use crate::core::memory::nest::address::RegionId;

/// 一段占用；`last` 是最后一个字节（含），generation 是 `id`。
#[derive(Clone, Copy)]
pub(super) struct Slot {
    pub(super) start: usize,
    pub(super) last: usize,
    pub(super) ticket: usize,
    pub(super) id: usize,
    pub(super) region: RegionId,
}
