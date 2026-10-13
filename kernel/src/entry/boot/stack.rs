use alloc::vec::Vec;
use riscv::register::satp;

use crate::core::memory::PAGE_SIZE;
use crate::core::memory::manager::{Frame, MapError, entry::PteFlags};
use crate::core::work::room::deadline;
use crate::core::work::unit::team::kernel;
use crate::machine::board;
use crate::machine::hart::{self, HartId};
use crate::machine::layout::{
    HART_FRAME_BASE, TRAP_STACK_GUARD, TRAP_STACK_SLOT_SHIFT, TRAP_STACK_SLOT_SIZE,
};
use crate::machine::switcher::context::TrapContext;
use crate::machine::switcher::stack::{self, TRAP_STACK_CANARY, trap_stack_base, trap_stack_edge};
use crate::machine::switcher::trampoline::check_fits_page;

pub(super) fn init() -> Result<(), MapError> {
    let segments = hart::hart_count();
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
    if board::info().free.size < need {
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
        let body_va = trap_stack_base(HartId::new(h));
        space
            .memory
            .map_boot_frames(body_va, frames, flags, flags)?;
        unsafe {
            (body_va.as_usize() as *mut usize).write(TRAP_STACK_CANARY);
        }
    }
    stack::set_boot_stacks(boot_stacks);
    check_fits_page();

    let ksatp = satp::read();
    for h in (0..hart::hart_count()).map(HartId::new) {
        let pa = space
            .memory
            .physical_at(HART_FRAME_BASE + h.get() * PAGE_SIZE)
            .expect("kernel frame not mapped");
        let frame = unsafe { &mut *(pa.as_usize() as *mut TrapContext) };
        frame.kernel_satp = ksatp;
        frame.kernel_sp = trap_stack_edge(h);
        frame.trap_handler = crate::core::memory::manager::addr::VirtAddr::from_raw(
            crate::entry::trap::trap_handler as *const () as usize,
        );
        frame.trap_stack_corrupt = TRAP_STACK_CANARY;
        frame.user_pa = pa;
        frame.user_satp = ksatp;
        frame.self_va = HART_FRAME_BASE + h.get() * PAGE_SIZE;
    }

    deadline::beat_until(deadline::blind_ceiling());
    stack::arm_hart();
    Ok(())
}
