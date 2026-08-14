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
use crate::memory::allocator::{InitResult, block, frame};

pub(crate) struct HybridAllocator;

impl HybridAllocator {
    pub const fn new() -> Self {
        Self
    }

    /// 初始化 block + frame 后端。
    ///
    /// block 先初始化（经 bump），frame 后初始化（经 bump），确保
    /// frame 的 base（= bump frontier）在所有 bump 分配之后，Link 节点不被覆盖。
    ///
    /// # Errors
    ///
    /// 任一后端初始化失败（`block::init` / `frame::init` 的错误原样传播，
    /// 已在对应模块附加上下文）。
    pub fn init(&self) -> InitResult<()> {
        block::init()?;
        frame::init()?;
        Ok(())
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

/// 初始化混合分配器（block + frame 后端）。
///
/// 必须在 `main` 早期调用恰好一次（经 `allocator::init`），在任何堆分配之前。
///
/// # Errors
///
/// 任一后端初始化失败，错误原样传播（见 [`HybridAllocator::init`]）。
pub fn init() -> InitResult<()> {
    HYBRID_ALLOCATOR.init()
}
