use core::sync::atomic::{AtomicPtr, AtomicUsize, Ordering};

use crate::layout::{HART_FRAME_BASE, MAX_HART_SLOTS};
use crate::memory::PAGE_SIZE;
use crate::memory::manager::addr::VirtAddr;
use crate::platform::machine;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HartId(usize);

impl HartId {
    pub const fn new(id: usize) -> Self {
        Self(id)
    }

    pub const fn get(self) -> usize {
        self.0
    }

    pub const fn bit(self) -> (usize, usize) {
        (
            self.0 / usize::BITS as usize,
            1usize << (self.0 % usize::BITS as usize),
        )
    }
}

impl core::fmt::Display for HartId {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        self.0.fmt(f)
    }
}

static STARTED_HARTS: AtomicUsize = AtomicUsize::new(1);

pub fn mark_hart_started(hart: HartId) {
    debug_assert!(
        hart.get() < MAX_HART_SLOTS,
        "hart id {hart} beyond MAX_HART_SLOTS {MAX_HART_SLOTS}"
    );
    STARTED_HARTS.fetch_max(hart.get() + 1, Ordering::Relaxed);
}

pub fn hart_count() -> usize {
    let n = machine::info().hart.count;
    assert!(
        n <= MAX_HART_SLOTS,
        "DTB reports {n} harts, at most {MAX_HART_SLOTS} VA slots"
    );
    n
}

#[inline]
pub fn hart_id() -> HartId {
    let id: usize;
    // SAFETY: 内核态 tp 恒为本 hart PerHart 指针
    unsafe {
        core::arch::asm!(
            "ld {0}, 0(tp)",
            out(reg) id,
            options(nomem, nostack, preserves_flags),
        );
    }
    HartId(id)
}

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
            scheduler: AtomicPtr::new(core::ptr::null_mut()),
            lease: AtomicUsize::new(crate::memory::manager::asid::vacant()),
            instruction_sync: AtomicUsize::new(1),
            _pad: [0; 3],
        }
    }
}

#[unsafe(no_mangle)]
static PER_HART: [PerHart; MAX_HART_SLOTS] = {
    let mut a: core::mem::MaybeUninit<[PerHart; MAX_HART_SLOTS]> = core::mem::MaybeUninit::uninit();
    let ptr = a.as_mut_ptr().cast::<PerHart>();
    let mut i = 0;
    while i < MAX_HART_SLOTS {
        // SAFETY: i < MAX_HART_SLOTS
        unsafe { ptr.add(i).write(PerHart::at(i)) };
        i += 1;
    }
    // SAFETY: MAX_HART_SLOTS 个元素已全部写入
    unsafe { a.assume_init() }
};

#[inline]
pub fn per_hart_ptr(id: HartId) -> usize {
    debug_assert!(
        id.get() < MAX_HART_SLOTS,
        "per_hart_ptr: id {id} beyond MAX_HART_SLOTS"
    );
    core::ptr::addr_of!(PER_HART[id.get()]) as usize
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

#[inline]
pub fn scheduler() -> *mut () {
    let p: usize;
    // SAFETY: 内核态 tp 恒为本 hart PerHart 指针
    unsafe {
        core::arch::asm!(
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
    // SAFETY: 内核态 tp 恒为本 hart PerHart 指针
    unsafe {
        core::arch::asm!(
            "ld {0}, 0x08(tp)",
            out(reg) f,
            options(nomem, nostack, preserves_flags),
        );
    }
    VirtAddr::wrap(f)
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
    assert!(core::mem::offset_of!(PerHart, id) == 0x00);
    assert!(core::mem::offset_of!(PerHart, frame) == 0x08);
    assert!(core::mem::offset_of!(PerHart, scheduler) == 0x10);
    assert!(core::mem::offset_of!(PerHart, lease) == 0x18);
    assert!(core::mem::offset_of!(PerHart, instruction_sync) == 0x20);
    assert!(core::mem::size_of::<PerHart>() == 64);
};
