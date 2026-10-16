//! 一份来源页上压着它的一个活关系。
//!
//! 这三样一起交出去，调用方就不必自己再推一遍窗口偏移，也不会拆成两次快照：
//! 认住关系的地址空间、关系本身，以及这一页落在它窗口里的哪个虚拟页。

use alloc::sync::Arc;

use crate::core::memory::address::VirtAddr;
use crate::core::memory::nest::{Nest, View};

/// 一个压着来源页的活关系。
///
/// `nest` 为 `None` 表示它认住的地址空间已失联：旧 TLB 无从核实，不能当成没事。
pub(crate) struct Pressing {
    pub(crate) nest: Option<Arc<Nest>>,
    pub(crate) view: Arc<View>,
    pub(crate) address: VirtAddr,
}
