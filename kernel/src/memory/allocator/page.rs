// 物理页帧分配器 — 为 MMU 子系统提供 4 KiB 对齐物理页帧
//
// 委托给 frame buddy 分配器，本身不管理任何物理内存范围。
// 分配时验证 4 KiB 对齐，清零页面以保证页表条目初始全为零。
//
// 架构层次：
//   bump (仅引导期) → frame (buddy, 运行时) → { block (≤4 KiB 小块), page (MMU 页表) }
//
// 并发安全：无内部状态（ZST），无锁。并发由底层的 frame 分配器 SpinLock 保证。

use core::alloc::{AllocError, Allocator, Layout};
use core::ptr::NonNull;

use crate::memory::PAGE_SIZE;

/// 页分配器 — 委托给 frame buddy 分配器的零尺寸包装。
///
/// 实现 `Allocator` trait，仅接受 `Layout::from_size_align(4096, 4096)`。
struct PageAllocator;

unsafe impl Allocator for PageAllocator {
    fn allocate(&self, layout: Layout) -> Result<NonNull<[u8]>, AllocError> {
        if layout.size() != PAGE_SIZE || layout.align() != PAGE_SIZE {
            return Err(AllocError);
        }
        let page = crate::memory::allocator::frame::allocator().allocate(layout)?;
        unsafe {
            core::ptr::write_bytes(page.as_ptr() as *mut u8, 0, PAGE_SIZE);
        }

        Ok(page)
    }

    unsafe fn deallocate(&self, ptr: NonNull<u8>, layout: Layout) {
        unsafe {
            crate::memory::allocator::frame::allocator().deallocate(ptr, layout);
        }
    }
}

static PAGE_ALLOCATOR: PageAllocator = PageAllocator;

pub fn allocator() -> &'static dyn Allocator {
    &PAGE_ALLOCATOR
}
