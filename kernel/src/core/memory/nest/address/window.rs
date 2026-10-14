use crate::core::memory::nest::address::RegionId;

/// 一个动态选址窗口；`last` 是最后一个可用地址（含）。
#[derive(Clone, Copy)]
pub(super) struct Window {
    pub(super) id: RegionId,
    pub(super) base: usize,
    pub(super) last: usize,
}
