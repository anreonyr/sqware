//! 用户堆：Talc 管理对象，页来源只调用现有内存接口。

use core::alloc::{GlobalAlloc, Layout};
use core::ptr::null_mut;

use spinning_top::RawSpinlock;
use talc::{TalcLock, source::GlobalAllocSource};

use crate::{PAGE_SIZE, core::adapt};

const BLOCK_SIZE: usize = 4 * PAGE_SIZE;

type Heap = TalcLock<RawSpinlock, GlobalAllocSource<Pages>>;

#[derive(Debug)]
struct Pages;

// SAFETY: 每个区域独立申请；Talc 保留原始地址和大小，整块归还。
unsafe impl GlobalAlloc for Pages {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        if layout.align() > PAGE_SIZE {
            return null_mut();
        }
        let Some(size) = layout.size().max(1).checked_next_multiple_of(PAGE_SIZE) else {
            return null_mut();
        };
        adapt::allocate(size).map_or(null_mut(), |addr| addr as *mut u8)
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        let size = layout.size().max(1).next_multiple_of(PAGE_SIZE);
        let _ = adapt::deallocate(ptr as usize, size);
    }
}

// 页来源不使用用户堆，避免获取或归还区域时递归进入 Talc。
#[cfg(target_arch = "riscv64")]
#[global_allocator]
static HEAP: Heap = Heap::new(GlobalAllocSource::with_block_size(Pages, BLOCK_SIZE));

#[cfg(test)]
#[path = "heap/tests.rs"]
mod tests;
