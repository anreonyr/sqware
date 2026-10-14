use alloc::alloc::{AllocError, Allocator};
use core::ptr::NonNull;

use erra::ResultExt;

use crate::core::memory::flat::allocator::{InitError, InitResult};
use crate::machine::sync::SpinLock;

#[path = "inner.rs"]
mod inner;

use inner::BumpInner;

pub(crate) struct BumpAllocator {
    inner: SpinLock<Option<BumpInner>>,
}

impl BumpAllocator {
    const fn new() -> Self {
        Self {
            inner: SpinLock::new(None),
        }
    }

    pub fn init(&self) -> Result<(), InitError> {
        let mut guard = self.inner.lock();
        let mut inner = BumpInner::new(0, 0, 0);
        inner.init()?;
        guard.replace(inner);
        Ok(())
    }
}

unsafe impl Allocator for BumpAllocator {
    fn allocate(
        &self,
        layout: core::alloc::Layout,
    ) -> Result<core::ptr::NonNull<[u8]>, AllocError> {
        let mut guard = self.inner.lock();
        let inner = guard.as_mut().ok_or(AllocError)?;

        let frontier = (inner.base + inner.used) as *mut u8;
        let next = unsafe { frontier.add(frontier.align_offset(layout.align())) };

        if next.addr() + layout.size() > inner.edge {
            return Err(AllocError);
        }

        inner.used = next.addr() - inner.base + layout.size();
        Ok(NonNull::slice_from_raw_parts(
            NonNull::new(next).ok_or(AllocError)?,
            layout.size(),
        ))
    }

    unsafe fn deallocate(&self, _ptr: core::ptr::NonNull<u8>, _layout: core::alloc::Layout) {}
}

pub fn boundary() -> usize {
    let guard = BUMP_ALLOCATOR.inner.lock();
    let inner = guard.as_ref().expect("bump allocator not initialized");
    inner.edge
}
pub fn frontier() -> usize {
    let guard = BUMP_ALLOCATOR.inner.lock();
    let inner = guard.as_ref().expect("bump allocator not initialized");
    inner.base + inner.used
}

pub(crate) static BUMP_ALLOCATOR: BumpAllocator = BumpAllocator::new();

pub fn allocator() -> &'static dyn Allocator {
    &BUMP_ALLOCATOR
}

pub fn init() -> InitResult<()> {
    BUMP_ALLOCATOR
        .init()
        .annotate("initializing bump allocator")
}
