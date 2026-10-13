use alloc::sync::Arc;
use alloc::vec::Vec;

use crate::core::memory::manager::addr::VirtAddr;
use crate::core::memory::manager::tree::Map;

/// 一段撤回的进度：要拆的映射、已开的退休票、游标。
pub(super) struct RangeReclaim {
    pub(super) maps: Vec<(Arc<Map>, VirtAddr, usize)>,
    pub(super) tickets: Vec<usize>,
    pub(super) cursor: usize,
}
