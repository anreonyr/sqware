use crate::core::memory::address::{PhysAddr, VirtAddr};
use crate::core::memory::nest::view::Access;

/// 一段连续帧：虚拟连续、物理连续，且落在同一个当前权限相同的区间内。
#[derive(Debug, Clone, Copy)]
pub(in crate::core::memory::nest) struct FrameRun {
    pub(in crate::core::memory::nest) start: VirtAddr,
    pub(in crate::core::memory::nest) physical: PhysAddr,
    pub(in crate::core::memory::nest) size: usize,
    pub(in crate::core::memory::nest) current: Access,
}
