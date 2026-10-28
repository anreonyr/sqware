use core::alloc::{AllocError, Layout};
use core::ptr::NonNull;

use crate::{InitError, Region};

/// Block 算法需要的外部责任：机器事实、帧页、统计、标注与分配禁令。
///
/// # Safety
///
/// 实现方返回的机器区域必须是实际独占的 arena；Bump/Frame 分配器必须与该 arena
/// 同一生命周期且稳定；`allocate_bump`/`allocate_prime` 的每次交付都匹配请求 `Layout`
/// 的尺寸与对齐、且是尚未交付的新鲜区间；`release_frame` 的调用点必须来自本分配器
/// 实际交付的 pool 页。该 effect 只由一次性的私有装台构造。
pub unsafe trait Effect: Sync {
    fn memory(&self) -> Region;
    fn hart_id(&self) -> usize;
    fn hart_count(&self) -> usize;
    fn install_block_kinds(&self, base: usize, len: usize) -> Result<(), InitError>;
    fn allocate_bump(&self, layout: Layout) -> Result<NonNull<[u8]>, AllocError>;
    fn allocate_prime(&self, layout: Layout) -> Result<NonNull<[u8]>, AllocError>;

    /// # Safety
    ///
    /// `ptr` 必须是本分配器此前从 Frame 取得、尚未归还的整页，`layout` 必须是当时的
    /// 页布局；不得重复归还或归还外来指针。
    unsafe fn release_frame(&self, ptr: NonNull<u8>, layout: Layout);

    fn record_block_take(&self, addr: usize, power: usize);
    fn record_block_give(&self, addr: usize, power: usize);
    fn record_pool_take(&self);
    fn record_pool_give(&self);
    fn assert_allocation_allowed(&self);
}
