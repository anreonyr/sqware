use core::alloc::{AllocError, Layout};
use core::ptr::NonNull;

/// Spare 算法需要的外部责任：trace 载荷尺寸、混合分配器的仓区、统计与分配禁令。
///
/// # Safety
///
/// 实现方提供的混合分配器必须在其整个生存期内稳定，并实际拥有 `allocate_spare`
/// 返回的仓区；仓区在 Spare 生存期内独占有效，交付匹配请求 `Layout` 的尺寸与对齐、
/// 且是尚未交付的新鲜区间。该 effect 只由一次性的私有装台构造。
pub unsafe trait Effect: Sync {
    fn trace_payload_bytes(&self) -> usize;
    fn allocate_spare(&self, layout: Layout) -> Result<NonNull<[u8]>, AllocError>;
    fn record_spare_take(&self, bytes: usize);
    fn record_spare_give(&self, bytes: usize);
    fn assert_allocation_allowed(&self);
}
