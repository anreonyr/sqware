//! 引用机制对内核的静态缝：每个入口只尝试一次，权威仍在 Kernel。

use env::{AcquireSource, FlatReference, MemoryResult, PieToken};

/// 来源持有的单次内核调用。
///
/// 取得失败不产生本地持有；释放失败保留同一句柄，重试责任在调用方。
pub trait ReferenceKernel {
    fn acquire(source: AcquireSource, retention: PieToken) -> MemoryResult<FlatReference>;
    fn release(reference: FlatReference) -> MemoryResult<()>;
}
