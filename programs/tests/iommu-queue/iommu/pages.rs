//! The pages the queue and the device share.
//!
//! Production takes these from a retained window (`execution::memory`) and publishes
//! the **real physical page starts** to the device while filling them through a local
//! view. A host process has no second mapping, so here a page is one page-aligned
//! buffer that stands for both: `at` is the address the queue writes through and
//! `physical` is the address the device reads and writes through. They are real
//! addresses because `regs` and the queue reach memory only through raw pointers.

use alloc::alloc::{Layout, alloc};
use alloc::vec::Vec;

/// One page: the unit of the window, and the unit `regs::page_number` shifts out.
pub const PAGE_SIZE: usize = 4096;

pub struct Pages {
    address: Vec<*mut u8>,
}

impl Pages {
    /// Take `count` zeroed, page-aligned pages.
    ///
    /// They are never released: a fixture owns the whole process for its test, and a
    /// freed page behind a device pointer is worse than a leak.
    pub fn take(count: usize) -> Self {
        let layout = Layout::from_size_align(PAGE_SIZE, PAGE_SIZE).expect("page layout");
        let mut address = Vec::with_capacity(count);
        for _ in 0..count {
            // SAFETY: the layout has a non-zero size and is a valid page layout.
            let page = unsafe { alloc(layout) };
            assert!(!page.is_null(), "the fixture needs a page");
            // SAFETY: `page` owns one page and outlives this call.
            unsafe { core::ptr::write_bytes(page, 0, PAGE_SIZE) };
            address.push(page);
        }
        Self { address }
    }

    /// How many pages this window holds.
    pub fn count(&self) -> usize {
        self.address.len()
    }

    /// Page `index` at byte `offset`, as this domain addresses it.
    pub fn at(&self, index: usize, offset: usize) -> *mut u8 {
        // SAFETY: `index` is inside the window; `offset` is the caller's business, and
        // every caller in this fixture keeps it inside the page.
        unsafe { self.address[index].add(offset) }
    }

    /// Page `index` as the device addresses it.
    pub fn physical(&self, index: usize) -> usize {
        self.address[index] as usize
    }
}
