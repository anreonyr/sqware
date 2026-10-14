use core::alloc::{AllocError, Allocator, Layout};
use core::ptr::NonNull;

use alloc::boxed::Box;
use erra::ResultExt;

use crate::core::memory::{
    PAGE_SIZE,
    allocator::{InitError, InitResult, hybrid},
};
use crate::machine::hart;
use crate::machine::sync::{Level, OnceLock, SpinLock};

#[path = "spare/blk.rs"]
mod blk;
#[path = "spare/inner.rs"]
mod inner;

use blk::Blk;
use inner::SpareInner;

const HEADER: usize = 32;
const MAX_ALIGN: usize = 16;

pub const DUMP_BUDGET: usize = 1024 * 1024;

pub struct SpareAllocator {
    inner: SpinLock<SpareInner>,
}

impl SpareAllocator {
    fn init() -> Result<Self, InitError> {
        let cap = {
            let payload = crate::core::work::record::trace::ring_bytes(hart::hart_count());
            (payload.next_multiple_of(MAX_ALIGN) + HEADER + DUMP_BUDGET).next_multiple_of(PAGE_SIZE)
        };
        let align = Layout::from_size_align(cap, PAGE_SIZE).map_err(|_| InitError::OutOfMemory)?;
        let chunk = crate::tag!(
            Spare,
            hybrid::allocator()
                .allocate(align)
                .map_err(|_| InitError::OutOfMemory)?
        );
        let base = chunk.as_ptr() as *mut u8 as usize;
        let edge = base + chunk.len();
        Ok(Self {
            inner: SpinLock::new_level(Level::Spare, SpareInner::new(base, edge)),
        })
    }

    pub(crate) fn total_bytes(&self) -> usize {
        let g = self.inner.lock();
        g.edge - g.base
    }
}

unsafe impl Allocator for SpareAllocator {
    fn allocate(&self, layout: Layout) -> Result<NonNull<[u8]>, AllocError> {
        super::assert_allocation_allowed();
        if layout.align() > MAX_ALIGN {
            return Err(AllocError);
        }
        let need = layout.size().max(1).next_multiple_of(MAX_ALIGN);
        let mut guard = self.inner.lock();
        let inner = &mut *guard;
        let Some(blk) = inner.pull(need) else {
            return Err(AllocError);
        };
        let addr = blk.as_ptr() as usize;
        let size = unsafe { blk.read() }.size;
        super::statistics::record_spare_take(size);
        Ok(NonNull::slice_from_raw_parts(
            NonNull::new((addr + HEADER) as *mut u8).ok_or(AllocError)?,
            layout.size(),
        ))
    }

    unsafe fn deallocate(&self, ptr: NonNull<u8>, _layout: Layout) {
        unsafe {
            let blk_addr = ptr.as_ptr() as usize - HEADER;
            let mut guard = self.inner.lock();
            let inner = &mut *guard;
            if !inner.owns(blk_addr) {
                return;
            }

            let blk = NonNull::new_unchecked(blk_addr as *mut Blk);
            let size = blk.read().size;
            super::statistics::record_spare_give(size);
            inner.push(blk);
        }
    }
}

static SPARE_ALLOCATOR: OnceLock<&'static SpareAllocator> = OnceLock::new();

pub fn spare() -> &'static SpareAllocator {
    SPARE_ALLOCATOR
        .get()
        .expect("spare allocator not initialized")
}

pub fn allocator() -> &'static dyn Allocator {
    SPARE_ALLOCATOR
        .get()
        .expect("spare allocator not initialized")
}

pub fn init() -> InitResult<()> {
    (|| -> Result<(), InitError> {
        let heap =
            Box::leak(Box::try_new(SpareAllocator::init()?).map_err(|_| InitError::OutOfMemory)?);
        SPARE_ALLOCATOR
            .set(heap)
            .map_err(|_| InitError::AlreadyInitialized)
    })()
    .annotate("initializing spare allocator")
}
