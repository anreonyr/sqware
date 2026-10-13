use alloc::alloc::Allocator;
use alloc::boxed::Box;
use alloc::sync::Arc;
use core::cell::UnsafeCell;

use crate::core::memory::PAGE_SIZE;

use super::MapError;

#[derive(Debug)]
pub struct Frame {
    page: Box<UnsafeCell<[u8; PAGE_SIZE]>, &'static dyn Allocator>,
}

impl Frame {
    pub fn allocate() -> Result<Arc<Self>, MapError> {
        let page = crate::tag!(Plain, unsafe {
            Box::try_new_zeroed_in(crate::core::memory::allocator::frame::allocator())
                .map_err(|_| MapError::OutOfMemory)?
                .assume_init()
        });
        Arc::try_new(Self { page }).map_err(|_| MapError::OutOfMemory)
    }

    pub fn physical_address(&self) -> usize {
        self.page.get() as usize
    }

    #[cfg(debug_assertions)]
    pub fn as_ptr(&self) -> *const u8 {
        self.page.get().cast()
    }

    pub fn as_mut_ptr(&self) -> *mut u8 {
        self.page.get().cast()
    }
}

unsafe impl Send for Frame {}
unsafe impl Sync for Frame {}
