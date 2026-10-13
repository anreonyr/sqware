use alloc::sync::Arc;
use alloc::vec::Vec;
use riscv::register::{satp, sie, stvec};

use crate::hart::HartId;
use crate::layout::{
    HART_FRAME_BASE, TRAP_STACK_BASE, TRAP_STACK_GUARD, TRAP_STACK_SLOT_SHIFT, TRAP_STACK_SLOT_SIZE,
};
use crate::lock::OnceLock;
use crate::memory::PAGE_SIZE;
use crate::memory::manager::{Frame, MapError};
use crate::memory::manager::addr::VirtAddr;
use crate::memory::manager::entry::PteFlags;
use crate::runtime::chrono::timer;
use crate::runtime::switcher::context::TrapContext;
use crate::runtime::switcher::trampoline::{alltraps_va, check_fits_page};
use crate::work::unit::team::kernel;

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
    if h >= crate::hart::hart_count() {
        return None;
    }
    let in_seg = off & (TRAP_STACK_SLOT_SIZE - 1);
    (in_seg > TRAP_STACK_GUARD && in_seg <= TRAP_STACK_SLOT_SIZE).then_some(HartId::new(h))
}

pub(crate) fn trap_stack_guard_hart(addr: usize) -> Option<HartId> {
    let off = addr.checked_sub(TRAP_STACK_BASE.as_usize())?;
    if off & (TRAP_STACK_SLOT_SIZE - 1) < TRAP_STACK_GUARD {
        let h = off >> TRAP_STACK_SLOT_SHIFT;
        (h < crate::hart::hart_count()).then_some(HartId::new(h))
    } else {
        None
    }
}

pub(crate) fn boot_stack(hart: usize) -> usize {
    BOOT_STACKS
        .get()
        .expect("boot stacks not initialized")[hart]
        .physical_address()
        + PAGE_SIZE
}

pub fn init() -> Result<(), MapError> {
    let segments = crate::hart::hart_count();
    assert!(segments > 0, "no harts");
    assert_eq!(
        TRAP_STACK_SLOT_SIZE,
        1 << TRAP_STACK_SLOT_SHIFT,
        "trap stack segment must be 2^SHIFT"
    );
    let body_size = TRAP_STACK_SLOT_SIZE - TRAP_STACK_GUARD;
    let body_pages = body_size / PAGE_SIZE;
    let pages_per_hart = body_pages + 1;
    let need = segments
        .checked_mul(pages_per_hart)
        .and_then(|pages| pages.checked_mul(PAGE_SIZE))
        .ok_or(MapError::OutOfMemory)?;
    if crate::platform::machine::info().free.size < need {
        return Err(MapError::OutOfMemory);
    }

    let mut boot_stacks = Vec::new();
    boot_stacks
        .try_reserve_exact(segments)
        .map_err(|_| MapError::OutOfMemory)?;
    let space = &kernel().expect("kernel team not initialized").space;
    let flags = PteFlags::V | PteFlags::R | PteFlags::W | PteFlags::A | PteFlags::D;
    for h in 0..segments {
        let boot = crate::tag!(TrapStack, Frame::allocate()?);
        boot_stacks.push(boot);

        let mut frames = Vec::new();
        frames
            .try_reserve_exact(body_pages)
            .map_err(|_| MapError::OutOfMemory)?;
        for _ in 0..body_pages {
            frames.push(crate::tag!(TrapStack, Frame::allocate()?));
        }
        let (body_va, _edge) = trap_stack_segment(h);
        space
            .tree
            .map_frames(body_va, frames, flags, flags)?;
        // The guard page has no mapping in the Tree.
        unsafe {
            (body_va.as_usize() as *mut usize).write(TRAP_STACK_CANARY);
        }
    }
    assert!(BOOT_STACKS.set(boot_stacks).is_ok(), "boot stacks double init");

    check_fits_page();

    let ksatp = satp::read();
    for h in (0..crate::hart::hart_count()).map(HartId::new) {
        let pa = kernel()
            .expect("kernel team not initialized")
            .space
            .tree
            .translate(HART_FRAME_BASE + h.get() * PAGE_SIZE)
            .expect("kernel frame not mapped")
            .0;
        let frame = unsafe { &mut *(pa.as_usize() as *mut TrapContext) };
        frame.kernel_satp = ksatp;
        frame.kernel_sp = trap_stack_edge(h);
        frame.trap_handler = VirtAddr::from_raw(super::trap_handler as *const () as usize);
        frame.trap_stack_corrupt = TRAP_STACK_CANARY;
        frame.user_pa = pa;
        frame.user_satp = ksatp;
        frame.self_va = HART_FRAME_BASE + h.get() * PAGE_SIZE;
    }

    timer::beat_until(timer::blind_ceiling());

    arm_hart();
    Ok(())
}

pub fn arm_hart() {
    unsafe {
        stvec::write(stvec::Stvec::new(alltraps_va(), stvec::TrapMode::Direct));
        let scr = crate::hart::hart_frame().as_usize();
        core::arch::asm!("csrw sscratch, {}", in(reg) scr);
        sie::set_stimer();
        sie::set_ssoft();
        sie::set_sext();
    }
}
