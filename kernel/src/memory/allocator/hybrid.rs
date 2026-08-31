// 混合路由分配器：按大小委派给 block 或 frame 后端。
//
//   layout.size() <= PAGE_SIZE/2 → block（多块页）
//   layout.size() >  PAGE_SIZE/2 → frame（order0 及以上）
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
    /// # Errors
    ///
    /// 任一后端初始化失败，错误原样传播。
    pub fn init(&self) -> InitResult<()> {
        block::init()?;
        frame::init()?;
        Ok(())
    }
}

unsafe impl Allocator for HybridAllocator {
    fn allocate(&self, layout: Layout) -> Result<NonNull<[u8]>, AllocError> {
        if layout.size() <= PAGE_SIZE / 2 {
            block::allocator().allocate(layout)
        } else {
            // 帧级默认 Persistent（全局容器缓冲/健康检查直取——手动标注：本处
            // 是分配器内部分流，装饰器面向业务分配点）。
            let p = frame::allocator().allocate(layout)?;
            #[cfg(feature = "audit")]
            super::fence::tag(
                p.as_ptr().cast::<u8>() as usize,
                super::fence::Class::Persistent,
            );
            Ok(p)
        }
    }

    unsafe fn deallocate(&self, ptr: NonNull<u8>, layout: Layout) {
        unsafe {
            if layout.size() <= PAGE_SIZE / 2 {
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

/// 初始化混合分配器（block + frame 后端）：在任何堆分配之前调用恰好一次。
pub fn init() -> InitResult<()> {
    HYBRID_ALLOCATOR.init()
}
