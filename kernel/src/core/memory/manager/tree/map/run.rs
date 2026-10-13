use crate::core::memory::manager::addr::{PhysAddr, VirtAddr};
use crate::core::memory::manager::entry::PteFlags;

/// 一段连续帧：虚拟连续、物理连续，且落在同一个当前权限相同的区间内。
#[derive(Debug, Clone, Copy)]
pub(in super::super) struct FrameRun {
    pub(in super::super) start: VirtAddr,
    pub(in super::super) physical: PhysAddr,
    pub(in super::super) size: usize,
    pub(in super::super) current: PteFlags,
}
