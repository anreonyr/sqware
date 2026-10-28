//! 宿主侧实际机制实例。对外只暴露 `block`/`frame` 的 dyn 分配器入口；
//! 装台、typed instance、spare/hybrid 与统计计数都是 crate 内部适配。

use crate::shim_lock::SpinLock;

pub use kernel_allocator::PAGE_SIZE;

pub(crate) type Lock = SpinLock<()>;

pub mod block;
pub mod frame;

pub(crate) mod bump;
pub(crate) mod hybrid;
pub(crate) mod spare;
pub(crate) mod statistics;
