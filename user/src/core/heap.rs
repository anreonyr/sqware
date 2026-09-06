//! 用户堆 — Global Alloc 后端 + `#[global_allocator]`。

use core::alloc::{GlobalAlloc, Layout};

use crate::PAGE_SIZE;
use crate::env::memory;

pub struct Heap;

/// 实际分配的字节（按页取整 + 至少一页）。
fn alloc_size(layout: &Layout) -> usize {
    layout.size().max(1).next_multiple_of(PAGE_SIZE)
}

unsafe impl GlobalAlloc for Heap {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        match memory::allocate(alloc_size(&layout)) {
            Ok(addr) => addr as *mut u8,
            Err(_) => core::ptr::null_mut(),
        }
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        let _ = memory::deallocate(ptr as usize, alloc_size(&layout));
    }

    /// realloc：只在确需跨页时分配新页 + 拷贝 + 释放旧页；否则原地返回。
    ///
    /// 系统分配是页粒度（`alloc`/`dealloc` 按页取整），而 `Vec`/`String` 的
    /// `new_size` 常是元素字节数的非页倍数。若 new_size 落在当前页内（原页已够），
    /// 返回原指针即可，避免每次扩容都换页（会加剧堆地址空间碎片 + 反复页分配）。
    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        if new_size <= alloc_size(&layout) {
            // 原页仍足够：原地返回，不搬移。
            return ptr;
        }
        // 需新页：alloc + copy + dealloc（同 GlobalAlloc 默认，但这里显式）。
        assert!(layout.size() != 0, "realloc: zero-size layout");
        assert!(new_size != 0, "realloc: zero new_size");
        let new_layout = unsafe {
            Layout::from_size_align_unchecked(new_size, layout.align())
        };
        let new_ptr = unsafe { self.alloc(new_layout) };
        if !new_ptr.is_null() {
            unsafe {
                core::ptr::copy_nonoverlapping(ptr, new_ptr, layout.size());
                self.dealloc(ptr, layout);
            }
        }
        new_ptr
    }
}

#[global_allocator]
static HEAP: Heap = Heap;
