use alloc::alloc::Allocator;
use alloc::boxed::Box;
use alloc::sync::Arc;
use core::cell::UnsafeCell;

use crate::core::memory::PAGE_SIZE;

use super::custody::Custody;
use super::error::FlatError;

#[derive(Debug)]
pub struct Frame {
    page: Box<UnsafeCell<[u8; PAGE_SIZE]>, &'static dyn Allocator>,
    custody: Custody,
}

impl Frame {
    pub fn allocate() -> Result<Arc<Self>, FlatError> {
        let page = crate::tag!(Plain, unsafe {
            Box::try_new_zeroed_in(super::allocator::frame::allocator())
                .map_err(|_| FlatError::OutOfMemory)?
                .assume_init()
        });
        Arc::try_new(Self {
            page,
            custody: Custody::new(),
        })
        .map_err(|_| FlatError::OutOfMemory)
    }

    /// 这份帧的迁移托管：只有 `Sole` 能开始移交。
    pub(super) fn custody(&self) -> &Custody {
        &self.custody
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
