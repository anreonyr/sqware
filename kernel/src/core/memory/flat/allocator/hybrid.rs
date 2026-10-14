use core::alloc::Layout;
use core::ptr::NonNull;

use alloc::alloc::{AllocError, Allocator};

use crate::core::memory::flat::allocator::{InitResult, block, frame};

pub(crate) struct HybridAllocator;

impl HybridAllocator {
    pub const fn new() -> Self {
        Self
    }

    pub fn init(&self) -> InitResult<()> {
        block::init()?;
        frame::init()?;
        Ok(())
    }
}

unsafe impl Allocator for HybridAllocator {
    fn allocate(&self, layout: Layout) -> Result<NonNull<[u8]>, AllocError> {
        super::assert_allocation_allowed();
        if layout.size() <= block::MAX_SIZE {
            block::allocator().allocate(layout)
        } else {
            frame::allocator().allocate(layout)
        }
    }

    #[track_caller]
    unsafe fn deallocate(&self, ptr: NonNull<u8>, layout: Layout) {
        unsafe {
            if layout.size() <= block::MAX_SIZE {
                block::allocator().deallocate(ptr, layout);
            } else {
                frame::allocator().deallocate(ptr, layout);
            }
        }
    }
}

pub(crate) static HYBRID_ALLOCATOR: HybridAllocator = HybridAllocator::new();

pub fn allocator() -> &'static dyn Allocator {
    &HYBRID_ALLOCATOR
}

pub fn init() -> InitResult<()> {
    HYBRID_ALLOCATOR.init()
}
