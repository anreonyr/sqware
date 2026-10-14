use crate::core::memory::PAGE_SIZE;
use core::ptr::NonNull;
use erra::ResultExt;

use alloc::{
    alloc::{AllocError, Allocator},
    boxed::Box,
};

use crate::core::memory::flat::allocator::{InitError, InitResult};
use crate::machine::sync::{Level, OnceLock, SpinLock};

#[path = "inner.rs"]
mod inner;

use inner::FrameInner;

pub(crate) struct FrameAllocator {
    inner: SpinLock<FrameInner>,
}

impl FrameAllocator {
    fn init() -> Result<Self, InitError> {
        let mut inner = FrameInner::new();
        inner.init()?;
        Ok(Self {
            inner: SpinLock::new_level(Level::Frame, inner),
        })
    }
}

fn block_power(size: usize) -> usize {
    size.max(PAGE_SIZE)
        .next_multiple_of(PAGE_SIZE)
        .next_power_of_two()
        .ilog2() as usize
        - PAGE_SIZE.ilog2() as usize
}

unsafe impl Allocator for FrameAllocator {
    fn allocate(&self, layout: core::alloc::Layout) -> Result<NonNull<[u8]>, AllocError> {
        super::assert_allocation_allowed();
        {
            let this = &self;
            let size = layout.size().max(PAGE_SIZE);
            let power = block_power(size);
            let mut guard = this.inner.lock();
            let frame = &mut *guard;
            let index = unsafe { frame.split_block(power) }.ok_or(AllocError)?;
            let addr = frame.frame_addr(index) as *mut u8;
            #[cfg(debug_assertions)]
            {
                let a = addr as usize;
                assert!(
                    (a >= frame.base) && (a < frame.edge),
                    "frame alloc out of range: {a:#x} not in [{:#x}, {:#x})",
                    frame.base,
                    frame.edge
                );
            }
            super::statistics::record_frame_take(index, power);
            Ok(NonNull::slice_from_raw_parts(
                NonNull::new(addr).ok_or(AllocError)?,
                size,
            ))
        }
    }

    #[track_caller]
    unsafe fn deallocate(&self, ptr: NonNull<u8>, layout: core::alloc::Layout) {
        unsafe {
            let mut guard = self.inner.lock();
            let frame = &mut *guard;
            let size = layout.size().max(PAGE_SIZE);
            let power = block_power(size);
            let addr = ptr.addr().get();
            #[cfg(debug_assertions)]
            {
                let a = addr;
                assert!(
                    (a >= frame.base) && (a < frame.edge),
                    "frame dealloc out of range: {a:#x} not in [{:#x}, {:#x})",
                    frame.base,
                    frame.edge
                );
            }
            let index = frame.frame_index(addr);
            super::statistics::record_frame_give(index);
            frame.merge_block(index, power);
        }
    }
}

static FRAME_ALLOCATOR: OnceLock<&'static FrameAllocator> = OnceLock::new();

pub fn allocator() -> &'static dyn Allocator {
    FRAME_ALLOCATOR
        .get()
        .expect("frame allocator not initialized")
}

pub fn init() -> InitResult<()> {
    (|| -> Result<(), InitError> {
        let mut slot =
            Box::<FrameAllocator>::try_new_uninit().map_err(|_| InitError::OutOfMemory)?;
        slot.write(FrameAllocator::init()?);
        let heap = Box::leak(unsafe { slot.assume_init() });
        FRAME_ALLOCATOR
            .set(heap)
            .map_err(|_| InitError::AlreadyInitialized)
    })()
    .annotate("initializing frame allocator")
}
