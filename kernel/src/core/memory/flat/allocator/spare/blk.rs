use core::ptr::NonNull;

use crate::core::memory::flat::allocator::Link;

use super::HEADER;

#[repr(C)]
pub(super) struct Blk {
    pub(super) link: Link,
    pub(super) size: usize,
}

impl Blk {
    pub(super) unsafe fn write(addr: usize, size: usize) -> NonNull<Blk> {
        let blk = unsafe { NonNull::new_unchecked(addr as *mut Blk) };
        unsafe {
            blk.write(Blk {
                link: Link::new(None, None),
                size,
            });
        }
        blk
    }

    pub(super) fn payload(&self) -> usize {
        self.size - HEADER
    }

    pub(super) fn next(&self) -> Option<NonNull<Blk>> {
        self.link.next.map(|n| n.cast())
    }
}
