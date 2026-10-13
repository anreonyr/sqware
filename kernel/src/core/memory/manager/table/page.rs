use alloc::alloc::Allocator;
use alloc::boxed::Box;

use crate::core::memory::manager::entry::PageTableEntry;

use super::error::MapError;

#[repr(C, align(4096))]
#[derive(Debug)]
pub(crate) struct PageTable {
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
    pub(crate) fn new() -> Result<Box<PageTable, &'static dyn Allocator>, MapError> {
        let page = crate::tag!(Table, unsafe {
            Box::try_new_zeroed_in(crate::core::memory::allocator::frame::allocator())
                .map_err(|_| MapError::OutOfMemory)?
                .assume_init()
        });
        super::TABLE_LIVE.fetch_add(1, core::sync::atomic::Ordering::Relaxed);
        Ok(page)
    }
}
