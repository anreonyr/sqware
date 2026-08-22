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

/// 每 hart 最近一次经门户分配的调用点（block-OOM 现场定位用：分配失败在
/// block，但业务调用点在 `__rust_alloc` 内联后的 portal 入口 ra 处）。per-hart
/// 槽避免跨核互踩；`inline(never)` + 入口首句捕获保证 ra 无中间调用覆盖。
static LAST_ALLOC_RA: [core::sync::atomic::AtomicUsize; 8] =
    [const { core::sync::atomic::AtomicUsize::new(0) }; 8];

/// 读取指定 hart 最后记录的门户分配调用点（OOM 诊断）。
pub(crate) fn last_alloc_ra(hart: usize) -> usize {
    LAST_ALLOC_RA
        .get(hart)
        .map(|a| a.load(core::sync::atomic::Ordering::Relaxed))
        .unwrap_or(0)
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
    /// 入口首句记录调用点（OOM 定位；`__rust_alloc` 通常内联进业务调用点，
    /// 此处 ra 即分配业务的返回地址）。
    #[inline(never)]
    fn allocate(&self, layout: Layout) -> Result<NonNull<[u8]>, AllocError> {
        // SAFETY: 读 ra 无副作用（同 lock/depend::ra 手法，此处无条件可用）。
        let call_ra: usize;
        unsafe { core::arch::asm!("mv {}, ra", out(reg) call_ra) };
        LAST_ALLOC_RA[crate::machine::hart_id()].store(
            call_ra,
            core::sync::atomic::Ordering::Relaxed,
        );
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
