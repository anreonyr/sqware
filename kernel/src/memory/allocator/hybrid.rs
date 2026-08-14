// 混合路由分配器 — 按大小委派给 block 或 frame 后端
//
// layout.size() <= PAGE_SIZE  → block::allocator()
// layout.size() >  PAGE_SIZE  → frame::allocator()
//
// hybrid 自身不管理任何内存，仅检查大小并路由。block 内部缺页时
// 直接调用 frame::allocator() 取页（锁序：block→frame，从不反向）。

use core::alloc::Layout;
use core::ptr::NonNull;

use alloc::alloc::{AllocError, Allocator};

use crate::memory::PAGE_SIZE;
use crate::memory::allocator::{block, frame};

pub(crate) struct HybridAllocator;

impl HybridAllocator {
    pub const fn new() -> Self {
        Self
    }

    /// 初始化 block + frame 后端。
    ///
    /// block 先初始化（经 bump），frame 后初始化（经 bump），确保
    /// frame 的 base（= bump frontier）在所有 bump 分配之后，Link 节点不被覆盖。
    pub fn init(&self) {
        block::init();
        frame::init();
    }
}

unsafe impl Allocator for HybridAllocator {
    fn allocate(&self, layout: Layout) -> Result<NonNull<[u8]>, AllocError> {
        if layout.size() <= PAGE_SIZE {
            block::allocator().allocate(layout)
        } else {
            frame::allocator().allocate(layout)
        }
    }

    unsafe fn deallocate(&self, ptr: NonNull<u8>, layout: Layout) {
        unsafe {
            if layout.size() <= PAGE_SIZE {
                block::allocator().deallocate(ptr, layout);
            } else {
                frame::allocator().deallocate(ptr, layout);
            }
        }
    }
}

pub(crate) static HYBRID_ALLOCATOR: HybridAllocator = HybridAllocator::new();

pub fn allocator() -> &'static dyn Allocator {
    &HYBRID_ALLOCATOR
}

pub fn init() {
    HYBRID_ALLOCATOR.init();
}
