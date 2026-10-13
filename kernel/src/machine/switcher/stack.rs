use alloc::sync::Arc;
use alloc::vec::Vec;
use riscv::register::{sie, stvec};

use crate::core::memory::PAGE_SIZE;
use crate::core::memory::manager::addr::VirtAddr;
use crate::core::memory::manager::Frame;
use crate::machine::hart::HartId;
use crate::machine::layout::{
    TRAP_STACK_BASE, TRAP_STACK_GUARD, TRAP_STACK_SLOT_SHIFT, TRAP_STACK_SLOT_SIZE,
};
use crate::machine::sync::OnceLock;
use crate::machine::switcher::trampoline::alltraps_va;

pub(crate) const TRAP_STACK_CANARY: usize = 0x5EED_CAFE_51A7_0000;

static BOOT_STACKS: OnceLock<Vec<Arc<Frame>>> = OnceLock::new();

fn trap_stack_segment(hart: usize) -> (VirtAddr, VirtAddr) {
    let base = TRAP_STACK_BASE.as_usize() + hart * TRAP_STACK_SLOT_SIZE;
    (
        VirtAddr::from_raw(base + TRAP_STACK_GUARD),
        VirtAddr::from_raw(base + TRAP_STACK_SLOT_SIZE),
    )
}

pub fn trap_stack_base(hart: HartId) -> VirtAddr {
    trap_stack_segment(hart.get()).0
}

pub fn trap_stack_edge(hart: HartId) -> VirtAddr {
    trap_stack_segment(hart.get()).1
}

pub(crate) fn trap_stack_hart(sp: usize) -> Option<HartId> {
    let off = sp.checked_sub(TRAP_STACK_BASE.as_usize())?;
    let h = off >> TRAP_STACK_SLOT_SHIFT;
    if h >= crate::machine::hart::hart_count() {
        return None;
    }
    let in_seg = off & (TRAP_STACK_SLOT_SIZE - 1);
    (in_seg > TRAP_STACK_GUARD && in_seg <= TRAP_STACK_SLOT_SIZE).then_some(HartId::new(h))
}

pub(crate) fn trap_stack_guard_hart(addr: usize) -> Option<HartId> {
    let off = addr.checked_sub(TRAP_STACK_BASE.as_usize())?;
    if off & (TRAP_STACK_SLOT_SIZE - 1) < TRAP_STACK_GUARD {
        let h = off >> TRAP_STACK_SLOT_SHIFT;
        (h < crate::machine::hart::hart_count()).then_some(HartId::new(h))
    } else {
        None
    }
}

pub(crate) fn boot_stack(hart: usize) -> usize {
    BOOT_STACKS.get().expect("boot stacks not initialized")[hart].physical_address() + PAGE_SIZE
}

pub(crate) fn set_boot_stacks(boot_stacks: Vec<Arc<Frame>>) {
    assert!(BOOT_STACKS.set(boot_stacks).is_ok(), "boot stacks double init");
}

pub fn arm_hart() {
    unsafe {
        stvec::write(stvec::Stvec::new(alltraps_va(), stvec::TrapMode::Direct));
        let scr = crate::machine::hart::hart_frame().as_usize();
        ::core::arch::asm!("csrw sscratch, {}", in(reg) scr);
        sie::set_stimer();
        sie::set_ssoft();
        sie::set_sext();
    }
}
