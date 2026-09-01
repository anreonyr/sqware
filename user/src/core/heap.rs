//! 用户堆 — Global Alloc 后端 + `#[global_allocator]`。

use core::alloc::{GlobalAlloc, Layout};

use crate::env::memory;
use crate::PAGE_SIZE;

pub struct Heap;

unsafe impl GlobalAlloc for Heap {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let size = layout.size().max(1).next_multiple_of(PAGE_SIZE);
        match memory::allocate(size) {
            Ok(addr) => addr as *mut u8,
            Err(_) => core::ptr::null_mut(),
        }
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        let size = layout.size().max(1).next_multiple_of(PAGE_SIZE);
        let _ = memory::deallocate(ptr as usize, size);
    }
}

#[global_allocator]
static HEAP: Heap = Heap;