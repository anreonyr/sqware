use core::alloc::{AllocError, Allocator, GlobalAllocator, Layout, StaticAllocator};
use core::ptr::NonNull;
use core::sync::atomic::{AtomicU8, Ordering};

use super::{bump, hybrid, spare};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
#[repr(u8)]
pub enum Backend {
    Bump = 1,
    Hybrid = 2,
    Spare = 3,
}

static BACKEND: AtomicU8 = AtomicU8::new(0);

fn backend() -> Option<&'static dyn Allocator> {
    match BACKEND.load(Ordering::Acquire) {
        b if b == Backend::Bump as u8 => Some(bump::allocator()),
        b if b == Backend::Hybrid as u8 => Some(hybrid::allocator()),
        b if b == Backend::Spare as u8 => Some(spare::allocator()),
        _ => None,
    }
}

pub struct PortalAllocator;

unsafe impl Sync for PortalAllocator {}

unsafe impl StaticAllocator for PortalAllocator {}

unsafe impl GlobalAllocator for PortalAllocator {}

unsafe impl Allocator for PortalAllocator {
    fn allocate(&self, layout: Layout) -> Result<NonNull<[u8]>, AllocError> {
        backend().ok_or(AllocError)?.allocate(layout)
    }

    unsafe fn deallocate(&self, ptr: NonNull<u8>, layout: Layout) {
        unsafe {
            if let Some(allocator) = backend() {
                allocator.deallocate(ptr, layout);
            }
        }
    }

    unsafe fn grow(
        &self,
        ptr: NonNull<u8>,
        old_layout: Layout,
        new_layout: Layout,
    ) -> Result<NonNull<[u8]>, AllocError> {
        let Some(allocator) = backend() else {
            return Err(AllocError);
        };
        // SAFETY: 默认 grow = 分配新块 → 拷贝 → 释放旧块
        unsafe { allocator.grow(ptr, old_layout, new_layout) }
    }
}

pub fn switch(backend: Backend) {
    BACKEND.store(backend as u8, Ordering::Release);
}

#[global_allocator]
pub static PORTAL_ALLOCATOR: PortalAllocator = PortalAllocator;
