use ::core::sync::atomic::{AtomicPtr, AtomicUsize, Ordering};

use crate::core::memory::PAGE_SIZE;
use crate::core::memory::address::VirtAddr;
use crate::machine::layout::{HART_FRAME_BASE, MAX_HART_SLOTS};

use super::{HartId, hart_count, hart_id};

#[repr(C, align(64))]
pub struct PerHart {
    pub id: usize,
    pub frame: VirtAddr,
    pub scheduler: AtomicPtr<()>,
    pub lease: AtomicUsize,
    pub(crate) instruction_sync: AtomicUsize,
    _pad: [usize; 3],
}

impl PerHart {
    const fn at(id: usize) -> Self {
        Self {
            id,
            frame: VirtAddr::wrap(HART_FRAME_BASE.as_usize() + id * PAGE_SIZE),
            scheduler: AtomicPtr::new(::core::ptr::null_mut()),
            lease: AtomicUsize::new(crate::core::memory::nest::identity::vacant()),
            instruction_sync: AtomicUsize::new(1),
            _pad: [0; 3],
        }
    }
}

#[unsafe(no_mangle)]
static PER_HART: [PerHart; MAX_HART_SLOTS] = {
    let mut a: ::core::mem::MaybeUninit<[PerHart; MAX_HART_SLOTS]> =
        ::core::mem::MaybeUninit::uninit();
    let ptr = a.as_mut_ptr().cast::<PerHart>();
    let mut i = 0;
    while i < MAX_HART_SLOTS {
        unsafe { ptr.add(i).write(PerHart::at(i)) };
        i += 1;
    }
    unsafe { a.assume_init() }
};

#[inline]
pub fn per_hart_ptr(id: HartId) -> usize {
    debug_assert!(
        id.get() < MAX_HART_SLOTS,
        "per_hart_ptr: id {id} beyond MAX_HART_SLOTS"
    );
    ::core::ptr::addr_of!(PER_HART[id.get()]) as usize
}

pub fn set_scheduler(id: HartId, p: *mut ()) {
    debug_assert!(
        id.get() < MAX_HART_SLOTS,
        "set_scheduler: id {id} beyond MAX_HART_SLOTS"
    );
    PER_HART[id.get()].scheduler.store(p, Ordering::Release);
}

pub(crate) fn request_instruction_sync() {
    for hart in &PER_HART[..hart_count()] {
        hart.instruction_sync.store(1, Ordering::Release);
    }
}

pub(crate) fn lease_load(hart: HartId) -> usize {
    debug_assert!(
        hart.get() < MAX_HART_SLOTS,
        "lease_load: hart {hart} beyond MAX_HART_SLOTS"
    );
    PER_HART[hart.get()].lease.load(Ordering::Acquire)
}

pub(crate) fn lease_store(value: usize) {
    let me = hart_id();
    PER_HART[me.get()].lease.store(value, Ordering::Release);
}

const _: () = {
    assert!(::core::mem::offset_of!(PerHart, id) == 0x00);
    assert!(::core::mem::offset_of!(PerHart, frame) == 0x08);
    assert!(::core::mem::offset_of!(PerHart, scheduler) == 0x10);
    assert!(::core::mem::offset_of!(PerHart, lease) == 0x18);
    assert!(::core::mem::offset_of!(PerHart, instruction_sync) == 0x20);
    assert!(::core::mem::size_of::<PerHart>() == 64);
};
