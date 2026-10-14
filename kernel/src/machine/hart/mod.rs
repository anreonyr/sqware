mod id;
mod per_hart;

pub use id::HartId;
pub use per_hart::{PerHart, per_hart_ptr, set_scheduler};
pub(crate) use per_hart::{lease_load, lease_store, request_instruction_sync};

use ::core::sync::atomic::{AtomicUsize, Ordering};

use crate::core::memory::address::VirtAddr;
use crate::machine::board;
use crate::machine::layout::MAX_HART_SLOTS;

static ONLINE: AtomicUsize = AtomicUsize::new(0);

pub(crate) fn mark_online() {
    ONLINE.fetch_or(hart_id().bit().1, Ordering::Release);
}

pub(crate) fn online(hart: HartId) -> bool {
    ONLINE.load(Ordering::Acquire) & hart.bit().1 != 0
}

pub fn hart_count() -> usize {
    let n = board::info().hart.count;
    assert!(
        n <= MAX_HART_SLOTS,
        "DTB reports {n} harts, at most {MAX_HART_SLOTS} VA slots"
    );
    n
}

#[inline]
pub fn hart_id() -> HartId {
    let id: usize;
    unsafe {
        ::core::arch::asm!(
            "ld {0}, 0(tp)",
            out(reg) id,
            options(nomem, nostack, preserves_flags),
        );
    }
    HartId::new(id)
}

#[inline]
pub fn scheduler() -> *mut () {
    let p: usize;
    unsafe {
        ::core::arch::asm!(
            "ld {0}, 0x10(tp)",
            out(reg) p,
            options(nomem, nostack, preserves_flags),
        );
    }
    p as *mut ()
}

#[inline]
pub fn hart_frame() -> VirtAddr {
    let f: usize;
    unsafe {
        ::core::arch::asm!(
            "ld {0}, 0x08(tp)",
            out(reg) f,
            options(nomem, nostack, preserves_flags),
        );
    }
    VirtAddr::wrap(f)
}
