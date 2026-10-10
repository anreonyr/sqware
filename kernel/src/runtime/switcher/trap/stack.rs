use riscv::register::{satp, sie, stvec};

use crate::hart::HartId;
use crate::layout::{
    HART_FRAME_BASE, TRAP_STACK_BASE, TRAP_STACK_GUARD, TRAP_STACK_SLOT_SHIFT, TRAP_STACK_SLOT_SIZE,
};
use crate::lock::OnceLock;
use crate::memory::PAGE_SIZE;
use crate::memory::manager::MapError;
use crate::memory::manager::addr::{PhysAddr, VirtAddr};
use crate::memory::manager::entry::PteFlags;
use crate::runtime::chrono::timer;
use crate::runtime::switcher::context::TrapContext;
use crate::runtime::switcher::trampoline::{alltraps_va, check_fits_page};
use crate::work::unit::team::kernel;

pub(crate) const TRAP_STACK_CANARY: usize = 0x5EED_CAFE_51A7_0000;

static TRAP_STACK_PHYS: OnceLock<usize> = OnceLock::new();

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

pub fn trap_stack() -> usize {
    *TRAP_STACK_PHYS.get().expect("trap stacks not initialized")
}

pub fn init() -> Result<(), MapError> {
    let per_hart = TRAP_STACK_SLOT_SIZE + PAGE_SIZE;
    let need = crate::hart::hart_count() * per_hart;
    if crate::platform::machine::info().free.size < need {
        return Err(MapError::OutOfMemory);
    }

    let segments = crate::hart::hart_count();
    assert!(segments > 0, "no harts");
    assert_eq!(
        TRAP_STACK_SLOT_SIZE,
        1 << TRAP_STACK_SLOT_SHIFT,
        "trap stack segment must be 2^SHIFT"
    );
    let total = segments * TRAP_STACK_SLOT_SIZE;
    let layout = core::alloc::Layout::from_size_align(total, PAGE_SIZE).expect("trap stack layout");
    let block = crate::tag!(
        TrapStack,
        crate::memory::allocator::frame::allocator()
            .allocate(layout)
            .map_err(|_| MapError::OutOfMemory)?
    );
    let base = block.cast::<u8>().as_ptr() as usize;
    assert!(
        TRAP_STACK_PHYS.set(base).is_ok(),
        "trap stack phys double init"
    );

    let space = &kernel().expect("kernel team not initialized").space;
    let flags = PteFlags::V | PteFlags::R | PteFlags::W | PteFlags::A | PteFlags::D;
    for h in 0..segments {
        let (body_va, _edge) = trap_stack_segment(h);
        let phys = base + h * TRAP_STACK_SLOT_SIZE;
        space.borrow(
            body_va,
            PhysAddr::from_raw(phys + TRAP_STACK_GUARD),
            TRAP_STACK_SLOT_SIZE - TRAP_STACK_GUARD,
            flags,
        )?;
        space.unmap(VirtAddr::from_raw(phys), TRAP_STACK_GUARD)?;
        unsafe {
            (body_va.as_usize() as *mut usize).write(TRAP_STACK_CANARY);
        }
    }

    check_fits_page();

    let ksatp = satp::read();
    for h in (0..crate::hart::hart_count()).map(HartId::new) {
        let pa = kernel()
            .expect("kernel team not initialized")
            .space
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
