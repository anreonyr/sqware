//! 用户堆 — `heap_allocate`/`heap_deallocate` envcall 后端 + `#[global_allocator]`。
//!
//! 内核 `Space::heap_allocate`（heap 窗口位图）就是真正的分配器：每次分配给出一块
//! 页对齐、已清零、互不重叠的独立区间，`heap_deallocate` 按 `(addr, 页对齐 size)`
//! 精确归还。本层只做「页对齐 + 转发」，是无状态 bump 语义（不在此层复用/合并）。
//!
//! 提供 `#[global_allocator]`：ubi 依赖的 fack-core 会 `extern crate alloc`，用户程序
//! 必须具备分配器；实际分配发生在用户代码真正用 `alloc` 时（demo 目前不分配）。

use core::alloc::{GlobalAlloc, Layout};

use crate::env;
use crate::PAGE_SIZE;

/// 转发型全局分配器：alloc = 内核堆 `heap_allocate`，dealloc = `heap_deallocate`。
///
/// 内核位图保证同尺寸分配互不重叠，故 alloc/dealloc 两端对同一 `layout.size()` 做
/// 相同页对齐即可精确匹配，无需在此层记录尺寸。
pub struct Heap;

unsafe impl GlobalAlloc for Heap {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let size = layout.size().max(1).next_multiple_of(PAGE_SIZE);
        match env::heap_allocate(size) {
            Ok(addr) => addr as *mut u8,
            Err(_) => core::ptr::null_mut(),
        }
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        let size = layout.size().max(1).next_multiple_of(PAGE_SIZE);
        let _ = env::heap_deallocate(ptr as usize, size);
    }
}

#[global_allocator]
static HEAP: Heap = Heap;
