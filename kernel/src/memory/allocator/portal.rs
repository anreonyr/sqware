// 内核分配器门户 — 通过 trait object 在不同启动阶段切换分配器实现
//
// PortalAllocator 作为 #[global_allocator]，内部持有 &dyn Allocator，
// 可在不同启动阶段委托给不同的分配器实例：
//   1. 初始阶段：None，任何分配返回 Err
//   2. 早期启动：委托给 bump 分配器
//   3. 运行时：委托给 frame 分配器
//
// 各后端分配器通过全局 static 提供 'static 引用，无需额外包装函数。

use core::alloc::Layout;
use core::ptr::NonNull;

use alloc::alloc::{AllocError, Allocator, GlobalAllocator};

use crate::lock::SpinLock;

pub struct PortalAllocator {
    inner: SpinLock<PortalInner>,
}

struct PortalInner {
    allocator: Option<&'static dyn Allocator>,
}

impl PortalAllocator {
    pub const fn new() -> Self {
        Self {
            inner: SpinLock::new(PortalInner { allocator: None }),
        }
    }
}

unsafe impl Sync for PortalAllocator {}

unsafe impl GlobalAllocator for PortalAllocator {}

unsafe impl Allocator for PortalAllocator {
    fn allocate(&self, layout: Layout) -> Result<NonNull<[u8]>, AllocError> {
        let inner = self.inner.lock();
        inner.allocator.ok_or(AllocError)?.allocate(layout)
    }

    unsafe fn deallocate(&self, ptr: NonNull<u8>, layout: Layout) {
        unsafe {
            let inner = self.inner.lock();
            if let Some(allocator) = inner.allocator {
                allocator.deallocate(ptr, layout);
            }
        }
    }
}

pub fn switch(allocator: &'static dyn Allocator) {
    let mut inner = PORTAL_ALLOCATOR.inner.lock();
    inner.allocator = Some(allocator);
}

#[global_allocator]
pub static PORTAL_ALLOCATOR: PortalAllocator = PortalAllocator::new();
