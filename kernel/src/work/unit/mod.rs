pub(crate) mod gate;
pub(crate) mod capsule;
pub(crate) mod life;
pub mod space;
pub(crate) mod task;
pub(crate) mod team;
pub(crate) mod weak;

use alloc::boxed::Box;
use alloc::sync::Arc;
use alloc::vec;
use erra::ResultExt;

use riscv::register::satp;

use crate::hart;
use crate::layout::kernel_edge;
use crate::memory::PAGE_SIZE;
use crate::memory::manager::{
    MapError,
    addr::{PhysAddr, VirtAddr},
    entry::PteFlags,
    flush_asid, mode,
};
use crate::platform::machine;

use crate::layout::{HART_FRAME_BASE, TRAMPOLINE, trampoline_pa};
use space::SpaceBuilder;

unsafe extern "C" {
    static _kernel_base: u8;
    static _text_end: u8;
    static _rodata_start: u8;
}

pub type MapResult<T> = erra::Result<T, MapError>;

pub(crate) fn build(
    kind: space::SpaceKind,
    sire: weak::TaskWeak,
) -> Result<Arc<team::Team>, MapError> {
    let space = match kind {
        space::SpaceKind::Supervisor => SpaceBuilder::supervisor(),
        space::SpaceKind::User => SpaceBuilder::user(),
    }.build()?;
    space.with(|inner| inner.dynamic(PAGE_SIZE));
    team::TeamBuilder::new(space).sire(sire).constructing().spawn()
}

pub fn init() -> MapResult<()> {
    (|| -> Result<(), MapError> {
        unsafe {
            let m = machine::info();

            mode::detect().unwrap_or_else(|e| panic!("satp mode detect failed: {e:?}"));

            if VirtAddr::from_raw(m.dram.base + m.dram.size) > mode::upper() {
                return Err(MapError::DramOverlap);
            }

            let kernel_space = SpaceBuilder::kernel().build()?;

            {
                let this = &kernel_space;
                let base = crate::layout::IMAGE_BASE.as_usize();
                this.with(|inner| {
                    inner.dynamic(base);
                    inner.allocate(space::SegmentKind::Normal, m.dram.base, m.dram.size)
                })?;
            };

            let ram_flags = PteFlags::V
                | PteFlags::R
                | PteFlags::W
                | PteFlags::A
                | PteFlags::D
                | PteFlags::G;

            let text_start = (&raw const _kernel_base).addr();
            let text_end = (&raw const _text_end).addr();
            let rodata_start = (&raw const _rodata_start).addr();
            let image_end = kernel_edge();
            let ram_end = m.dram.base.checked_add(m.dram.size).ok_or(MapError::NoRegion)?;
            if !(m.dram.base <= text_start && text_start < text_end
                && text_end <= rodata_start && rodata_start <= image_end && image_end <= ram_end) {
                return Err(MapError::NoRegion);
            }
            let ro_flags = PteFlags::V | PteFlags::R | PteFlags::A | PteFlags::G;
            let text_flags = ro_flags | PteFlags::X;
            for (start, end, flags) in [
                (m.dram.base, text_start, ram_flags),
                (text_start, text_end, text_flags),
                (text_end, rodata_start, ram_flags),
                (rodata_start, image_end, ro_flags),
                (image_end, ram_end, ram_flags),
            ] {
                if start == end { continue; }
                for offset in [0, mode::lower().as_usize()] {
                    kernel_space.borrow(
                        VirtAddr::wrap(offset + start),
                        PhysAddr::from_raw(start),
                        end - start,
                        flags,
                    )?;
                }
            }

            let tramp_flags =
                PteFlags::V | PteFlags::R | PteFlags::X | PteFlags::A | PteFlags::D | PteFlags::G;
            kernel_space.borrow(TRAMPOLINE, trampoline_pa(), PAGE_SIZE, tramp_flags)?;

            let n = hart::hart_count();
            for h in 0..n {
                let page: crate::memory::manager::table::Frame = crate::tag!(HartFrame, {
                    Box::try_new_zeroed_in(crate::memory::allocator::frame::allocator())
                        .map_err(|_| MapError::OutOfMemory)?
                        .assume_init()
                });
                kernel_space.attach(
                    HART_FRAME_BASE + h * PAGE_SIZE,
                    vec![page],
                    PteFlags::V | PteFlags::R | PteFlags::W | PteFlags::A | PteFlags::D,
                )?;
            }

            satp::set(mode::mode(), kernel_space.asid().get(), kernel_space.root());

            flush_asid(kernel_space.asid());
            #[cfg(debug_assertions)]
            crate::layout::validate();

            team::init_kernel(Arc::new(kernel_space));

            Ok(())
        }
    })()
    .annotate("initializing unit (kernel space + team)")
}
