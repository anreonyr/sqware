use core::alloc::{AllocError, Allocator, Layout};
use core::ptr::NonNull;

use crate::{InitError, Lock, LockRole, Locked};
use env::PAGE_SIZE;

mod effect;
#[path = "blk.rs"]
mod blk;
#[path = "inner.rs"]
mod inner;

use blk::Blk;
pub use effect::Effect;
use inner::SpareInner;

const HEADER: usize = 32;
const MAX_ALIGN: usize = 16;

pub const DUMP_BUDGET: usize = 1024 * 1024;

pub struct SpareAllocator<L: Lock, E: Effect> {
    effect: E,
    inner: Locked<L, SpareInner>,
}

impl<L: Lock, E: Effect> SpareAllocator<L, E> {
    pub fn new(effect: E) -> Result<Self, InitError> {
        let cap = {
            let payload = effect.trace_payload_bytes();
            (payload.next_multiple_of(MAX_ALIGN) + HEADER + DUMP_BUDGET).next_multiple_of(PAGE_SIZE)
        };
        let align = Layout::from_size_align(cap, PAGE_SIZE).map_err(|_| InitError::OutOfMemory)?;
        let chunk = effect
            .allocate_spare(align)
            .map_err(|_| InitError::OutOfMemory)?;
        let base = chunk.as_ptr() as *mut u8 as usize;
        let edge = base + chunk.len();
        Ok(Self {
            effect,
            inner: Locked::new(L::new(Some(LockRole::Spare)), SpareInner::new(base, edge)),
        })
    }

    pub fn total_bytes(&self) -> usize {
        self.inner.with(|g| g.edge - g.base)
    }
}

unsafe impl<L: Lock, E: Effect> Allocator for SpareAllocator<L, E> {
    fn allocate(&self, layout: Layout) -> Result<NonNull<[u8]>, AllocError> {
        self.effect.assert_allocation_allowed();
        if layout.align() > MAX_ALIGN {
            return Err(AllocError);
        }
        let need = layout.size().max(1).next_multiple_of(MAX_ALIGN);
        self.inner.with(|inner| {
            let Some(blk) = inner.pull(need) else {
                return Err(AllocError);
            };
            let addr = blk.as_ptr() as usize;
            let size = unsafe { blk.read() }.size;
            self.effect.record_spare_take(size);
            Ok(NonNull::slice_from_raw_parts(
                NonNull::new((addr + HEADER) as *mut u8).ok_or(AllocError)?,
                layout.size(),
            ))
        })
    }

    unsafe fn deallocate(&self, ptr: NonNull<u8>, _layout: Layout) {
        unsafe {
            let blk_addr = ptr.as_ptr() as usize - HEADER;
            self.inner.with(|inner| {
                if !inner.owns(blk_addr) {
                    return;
                }

                let blk = NonNull::new_unchecked(blk_addr as *mut Blk);
                let size = blk.read().size;
                self.effect.record_spare_give(size);
                inner.push(blk);
            })
        }
    }
}
