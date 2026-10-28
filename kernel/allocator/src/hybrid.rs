use core::alloc::{AllocError, Allocator, Layout};
use core::ptr::NonNull;

use crate::block;

/// Hybrid 分流需要同一对稳定 Block/Frame 实例，以及实际分配禁令。
///
/// # Safety
///
/// `block`/`frame` 必须在整个 Hybrid 生存期内返回同一对实际实例；返回的分配器必须
/// 真正实现 `Sync` 并拥有对应 arena，分流后的释放仍回到同一后端；每次交付匹配请求
/// `Layout` 的尺寸与对齐、且是尚未交付的新鲜区间。该 effect 只由一次性的私有装台构造。
pub unsafe trait Effect: Sync {
    fn block(&self) -> &'static (dyn Allocator + Sync);
    fn frame(&self) -> &'static (dyn Allocator + Sync);
    fn assert_allocation_allowed(&self);
}

pub struct HybridAllocator<E: Effect> {
    effect: E,
}

impl<E: Effect> HybridAllocator<E> {
    pub const fn new(effect: E) -> Self {
        Self { effect }
    }
}

unsafe impl<E: Effect> Allocator for HybridAllocator<E> {
    fn allocate(&self, layout: Layout) -> Result<NonNull<[u8]>, AllocError> {
        self.effect.assert_allocation_allowed();
        if layout.size() <= block::MAX_SIZE {
            self.effect.block().allocate(layout)
        } else {
            self.effect.frame().allocate(layout)
        }
    }

    #[track_caller]
    unsafe fn deallocate(&self, ptr: NonNull<u8>, layout: Layout) {
        unsafe {
            if layout.size() <= block::MAX_SIZE {
                self.effect.block().deallocate(ptr, layout);
            } else {
                self.effect.frame().deallocate(ptr, layout);
            }
        }
    }
}
