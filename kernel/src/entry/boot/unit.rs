use alloc::sync::Arc;
use alloc::vec::Vec;

use erra::ResultExt;

use riscv::register::satp;

use crate::machine::hart;
use crate::machine::layout::{HART_FRAME_BASE, TRAMPOLINE, trampoline_pa};
use crate::machine::image::kernel_edge;
use crate::core::memory::PAGE_SIZE;
use crate::core::memory::manager::{
    BorrowedRegion, Frame, MapError,
    addr::{PhysAddr, VirtAddr},
    entry::PteFlags,
    flush_asid, mode,
};
use crate::machine::board;
use crate::core::memory::manager::RegionId;
use crate::core::work::unit::team;
use crate::core::work::unit::space::SpaceBuilder;

use crate::core::work::unit::MapResult;

unsafe extern "C" {
    static _kernel_base: u8;
    static _text_end: u8;
    static _rodata_start: u8;
}

pub fn init() -> MapResult<()> {
    (|| -> Result<(), MapError> {
        unsafe {
            let m = board::info();

            if VirtAddr::from_raw(m.dram.base + m.dram.size) > mode::upper() {
                return Err(MapError::DramOverlap);
            }

            let kernel_space = SpaceBuilder::kernel().build()?;
            let _identity_ram = kernel_space.memory.reserve(
                RegionId::Normal,
                VirtAddr::from_raw(m.dram.base),
                m.dram.size,
            )?;

            let ram_flags =
                PteFlags::V | PteFlags::R | PteFlags::W | PteFlags::A | PteFlags::D | PteFlags::G;

            let text_start = (&raw const _kernel_base).addr();
            let text_end = (&raw const _text_end).addr();
            let rodata_start = (&raw const _rodata_start).addr();
            let image_end = kernel_edge();
            let ram_end = m
                .dram
                .base
                .checked_add(m.dram.size)
                .ok_or(MapError::NoRegion)?;
            if !(m.dram.base <= text_start
                && text_start < text_end
                && text_end <= rodata_start
                && rodata_start <= image_end
                && image_end <= ram_end)
            {
                return Err(MapError::NoRegion);
            }
            kernel_space.memory.advance_region(RegionId::Normal, image_end)?;
            let ro_flags = PteFlags::V | PteFlags::R | PteFlags::A | PteFlags::G;
            let text_flags = ro_flags | PteFlags::X;
            for (start, end, flags) in [
                (m.dram.base, text_start, ram_flags),
                (text_start, text_end, text_flags),
                (text_end, rodata_start, ram_flags),
                (rodata_start, image_end, ro_flags),
                (image_end, ram_end, ram_flags),
            ] {
                if start == end {
                    continue;
                }
                for offset in [0, mode::lower().as_usize()] {
                    let size = end - start;
                    let region = BorrowedRegion::new(PhysAddr::from_raw(start), size)?;
                    kernel_space.memory.map_boot_borrowed(
                        VirtAddr::wrap(offset + start),
                        region,
                        0,
                        size,
                        flags,
                        flags,
                    )?;
                }
            }

            let tramp_flags =
                PteFlags::V | PteFlags::R | PteFlags::X | PteFlags::A | PteFlags::D | PteFlags::G;
            let trampoline = BorrowedRegion::new(trampoline_pa(), PAGE_SIZE)?;
            kernel_space.memory.map_boot_borrowed(
                TRAMPOLINE,
                trampoline,
                0,
                PAGE_SIZE,
                tramp_flags,
                tramp_flags,
            )?;

            let n = hart::hart_count();
            for h in 0..n {
                let frame = crate::tag!(HartFrame, Frame::allocate()?);
                let mut frames = Vec::new();
                frames
                    .try_reserve_exact(1)
                    .map_err(|_| MapError::OutOfMemory)?;
                frames.push(frame);
                let flags = PteFlags::V | PteFlags::R | PteFlags::W | PteFlags::A | PteFlags::D;
                kernel_space.memory.map_boot_frames(
                    HART_FRAME_BASE + h * PAGE_SIZE,
                    frames,
                    flags,
                    flags,
                )?;
            }

            let kernel_space = Arc::try_new(kernel_space).map_err(|_| MapError::OutOfMemory)?;
            team::init_kernel(kernel_space.clone())?;
            hart::mark_online();
            satp::set(
                mode::mode(),
                kernel_space.memory.asid().get(),
                kernel_space.memory.root_ppn(),
            );

            flush_asid(kernel_space.memory.asid());
            #[cfg(debug_assertions)]
            crate::machine::layout::validate();

            Ok(())
        }
    })()
    .annotate("initializing unit (kernel space + team)")
}
