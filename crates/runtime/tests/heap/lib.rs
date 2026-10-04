#![cfg(test)]

pub const PAGE_SIZE: usize = 4096;

mod core {
    pub mod adapt {
        use std::alloc::{GlobalAlloc, Layout, System};
        use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

        pub static CALLS: AtomicUsize = AtomicUsize::new(0);
        pub static BYTES: AtomicUsize = AtomicUsize::new(0);
        pub static FAIL: AtomicBool = AtomicBool::new(false);

        pub fn allocate(size: usize) -> Result<usize, ()> {
            assert_eq!(size % crate::PAGE_SIZE, 0);
            if FAIL.load(Ordering::Relaxed) { return Err(()); }
            let layout = Layout::from_size_align(size, crate::PAGE_SIZE).unwrap();
            // SAFETY: the page layout is valid and System is independent of Talc.
            let ptr = unsafe { System.alloc(layout) };
            if ptr.is_null() { return Err(()); }
            CALLS.fetch_add(1, Ordering::Relaxed);
            BYTES.fetch_add(size, Ordering::Relaxed);
            Ok(ptr as usize)
        }

        pub fn deallocate(addr: usize, size: usize) -> Result<(), ()> {
            let layout = Layout::from_size_align(size, crate::PAGE_SIZE).unwrap();
            // SAFETY: Talc returns the original region and matching size.
            unsafe { System.dealloc(addr as *mut u8, layout) };
            BYTES.fetch_sub(size, Ordering::Relaxed);
            Ok(())
        }
    }
}

#[path = "../../src/core/task/heap.rs"]
mod heap;
