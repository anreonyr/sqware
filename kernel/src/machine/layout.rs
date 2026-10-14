use crate::core::memory::PAGE_SIZE;
use crate::core::memory::address::{PhysAddr, VirtAddr};
#[cfg(debug_assertions)]
use crate::machine::mode;

pub const IMAGE_BASE: VirtAddr = VirtAddr::wrap(0x1_0000);

pub(crate) const TASK_STACK_SIZE: usize = 64 * 1024;
pub(crate) const TASK_STACK_GUARD: usize = PAGE_SIZE;

pub const MAX_HART_SLOTS: usize = 4096;

pub(crate) const TRAMPOLINE: VirtAddr = VirtAddr::wrap(0xFFFF_FFFF_FFFF_F000);

pub(crate) const KERNEL_TOP: VirtAddr =
    VirtAddr::wrap(TRAMPOLINE.as_usize() - (2 * 1024 * 1024 - PAGE_SIZE));

pub(crate) fn trampoline_pa() -> PhysAddr {
    unsafe extern "C" {
        static __trampoline_start: u8;
    }
    PhysAddr::from_raw(::core::ptr::addr_of!(__trampoline_start) as usize)
}

pub(crate) const HART_FRAME_SLOTS: usize = MAX_HART_SLOTS;
pub(crate) const HART_FRAME_BASE: VirtAddr =
    VirtAddr::wrap(KERNEL_TOP.as_usize() - HART_FRAME_SLOTS * PAGE_SIZE);
pub(crate) const TEAM_FRAME_WINDOW_SIZE: usize = 64 * 1024 * 1024;
pub(crate) const TEAM_FRAME_BASE: VirtAddr =
    VirtAddr::wrap(HART_FRAME_BASE.as_usize() - TEAM_FRAME_WINDOW_SIZE);

pub(crate) const TRAP_STACK_SLOT_SIZE: usize = 64 * 1024;
pub(crate) const TRAP_STACK_SLOT_SHIFT: usize = 16;
pub(crate) const TRAP_STACK_GUARD: usize = PAGE_SIZE;
pub(crate) const TRAP_STACK_BASE: VirtAddr =
    VirtAddr::wrap(TEAM_FRAME_BASE.as_usize() - (MAX_HART_SLOTS << TRAP_STACK_SLOT_SHIFT));

const _: () = {
    assert!(TRAMPOLINE.as_usize().is_multiple_of(PAGE_SIZE));
    assert!(HART_FRAME_BASE.as_usize().is_multiple_of(PAGE_SIZE));
    assert!(KERNEL_TOP.as_usize().is_multiple_of(2 * 1024 * 1024));
    assert!(TRAMPOLINE.as_usize() - KERNEL_TOP.as_usize() == 2 * 1024 * 1024 - PAGE_SIZE);
    assert!(HART_FRAME_BASE.as_usize() + HART_FRAME_SLOTS * PAGE_SIZE == KERNEL_TOP.as_usize());
    assert!(HART_FRAME_BASE.as_usize().is_multiple_of(2 * 1024 * 1024));
    assert!(TEAM_FRAME_WINDOW_SIZE.is_multiple_of(2 * 1024 * 1024));
    assert!(TEAM_FRAME_BASE.as_usize().is_multiple_of(2 * 1024 * 1024));
    assert!(TEAM_FRAME_BASE.as_usize() + TEAM_FRAME_WINDOW_SIZE == HART_FRAME_BASE.as_usize());
    assert!(TASK_STACK_SIZE.is_multiple_of(PAGE_SIZE));
    assert!(TRAP_STACK_BASE.as_usize().is_multiple_of(2 * 1024 * 1024));
    assert!(
        TRAP_STACK_BASE.as_usize() + (MAX_HART_SLOTS << TRAP_STACK_SLOT_SHIFT)
            == TEAM_FRAME_BASE.as_usize()
    );
    assert!(TRAP_STACK_SLOT_SIZE == 1usize << TRAP_STACK_SLOT_SHIFT);
    assert!(TRAP_STACK_GUARD == PAGE_SIZE);
};

#[cfg(debug_assertions)]
pub(crate) fn validate() {
    let geo = mode::geometry(mode::mode());
    let split = geo.split_bit() as usize;
    let top = 1usize << split;
    let lower = mode::lower();
    let upper = mode::upper();
    assert!(
        (3..=5).contains(&geo.levels) && geo.va_bits as usize == 12 + 9 * geo.levels as usize,
        "mode geometry incoherent: {geo:?}"
    );
    assert_eq!(
        lower.as_usize(),
        (1usize << split) | (usize::MAX << (split + 1)),
        "lower not canonical kernel base"
    );
    assert!(!lower.is_user());
    assert_eq!(upper.as_usize(), top, "upper must equal user space ceiling");
    assert!(upper.as_usize().is_multiple_of(PAGE_SIZE));
    assert!(VirtAddr::wrap(upper.as_usize() - PAGE_SIZE).is_user());
    assert!(!TRAMPOLINE.is_user());
    assert!(HART_FRAME_BASE.as_usize() < TRAMPOLINE.as_usize());
    assert!(!TEAM_FRAME_BASE.is_user());
    assert!(!TRAP_STACK_BASE.is_user());
    assert!(
        TRAP_STACK_BASE.as_usize() + (MAX_HART_SLOTS << TRAP_STACK_SLOT_SHIFT)
            == TEAM_FRAME_BASE.as_usize()
    );
}
