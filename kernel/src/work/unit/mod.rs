pub(crate) mod gate;
pub(crate) mod life;
pub(crate) mod loader;
pub(crate) mod parser;
pub(crate) mod source;
pub mod space;
pub(crate) mod task;
pub(crate) mod team;
pub(crate) mod weak;

use alloc::alloc::Allocator;
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
    static _rodata_start: u8;
}

pub type MapResult<T> = erra::Result<T, MapError>;

pub(crate) fn build(
    source: &source::Source,
    kind: space::SpaceKind,
    sire: weak::TaskWeak,
) -> Result<Arc<team::Team>, team::UnitError> {
    let mut head: Box<[u8; source::HEAD], &'static dyn Allocator> = unsafe {
        Box::try_new_zeroed_in(crate::memory::allocator::frame::allocator())
            .map_err(|_| team::UnitError::OoM)?
            .assume_init()
    };
    let n = source.len().min(source::HEAD);
    if !source.read(0, &mut head[..n]) {
        return Err(team::UnitError::Unreadable);
    }
    let parsed = parser::parse(&head[..n], source.len()).map_err(|_| team::UnitError::Load)?;
    let builder = match kind {
        space::SpaceKind::Supervisor => SpaceBuilder::supervisor(),
        space::SpaceKind::User => SpaceBuilder::user(),
    };
    let space = builder.build().map_err(|_| team::UnitError::Load)?;
    let loaded = loader::load(space, source, &parsed).map_err(|e| match e {
        loader::LoadError::Unreadable => team::UnitError::Unreadable,
        loader::LoadError::Map(MapError::OutOfMemory) => team::UnitError::OoM,
        loader::LoadError::Map(_) => team::UnitError::Load,
    })?;
    let entry = loaded.entry;
    let team = team::TeamBuilder::new(loaded.space)
        .sire(sire)
        .spawn()
        .map_err(|_| team::UnitError::Load)?;
    team.set_default_entry(entry.as_usize());
    Ok(team)
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
                this.with(|inner| inner.dynamic(base));
            };

            let ram_flags = PteFlags::V
                | PteFlags::R
                | PteFlags::W
                | PteFlags::X
                | PteFlags::A
                | PteFlags::D
                | PteFlags::G;

            kernel_space.borrow(
                VirtAddr::from_raw(m.dram.base),
                PhysAddr::from_raw(m.dram.base),
                m.dram.size,
                ram_flags,
            )?;

            kernel_space.borrow(
                mode::lower() + m.dram.base,
                PhysAddr::from_raw(m.dram.base),
                m.dram.size,
                ram_flags,
            )?;

            let rodata_start = (&raw const _rodata_start).addr();
            let rodata_size = kernel_edge() - rodata_start;
            let ro_flags = PteFlags::V | PteFlags::R | PteFlags::A | PteFlags::D | PteFlags::G;
            kernel_space.protect(VirtAddr::from_raw(rodata_start), rodata_size, ro_flags)?;
            kernel_space.protect(mode::lower() + rodata_start, rodata_size, ro_flags)?;

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