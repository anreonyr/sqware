use alloc::alloc::Allocator;
use alloc::boxed::Box;

use super::entry::PageTableEntry;
use crate::core::memory::nest::NestError;

#[repr(C, align(4096))]
#[derive(Debug)]
pub(super) struct PageTable {
    pub(super) entries: [PageTableEntry; 512],
}

impl Default for PageTable {
    fn default() -> Self {
        Self {
            entries: [PageTableEntry::default(); 512],
        }
    }
}

impl PageTable {
    pub(super) fn new() -> Result<Box<PageTable, &'static dyn Allocator>, NestError> {
        let page = crate::tag!(Table, unsafe {
            Box::try_new_zeroed_in(crate::core::memory::flat::allocator::frame::allocator())
                .map_err(|_| NestError::OutOfMemory)?
                .assume_init()
        });
        super::TABLE_LIVE.fetch_add(1, core::sync::atomic::Ordering::Relaxed);
        Ok(page)
    }
}
