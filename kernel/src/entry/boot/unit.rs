use alloc::sync::Arc;
use alloc::vec::Vec;

use erra::ResultExt;

use riscv::register::satp;

use crate::machine::hart;
use crate::machine::layout::{HART_FRAME_BASE, TRAMPOLINE, trampoline_pa};
use crate::machine::image::kernel_edge;
use crate::core::memory::PAGE_SIZE;
use crate::core::memory::address::{PhysAddr, VirtAddr};
use crate::core::memory::flat::{BorrowedRegion, Frame};
use crate::core::memory::nest::table::PteFlags;
use crate::core::memory::nest::{Access, NestError, RegionId, identity};
use crate::machine::mode;
use crate::machine::board;
use crate::core::work::unit::team;
use crate::core::work::unit::space::SpaceBuilder;

use crate::core::work::unit::NestResult;

unsafe extern "C" {
    static _kernel_base: u8;
    static _text_end: u8;
    static _rodata_start: u8;
}

/// 启动静态布局：低 identity 在 `Normal` 内按段挂载，高镜像、跳板、
/// hart frame 与 trap stack 按 `Exact` 登记真实占用。
pub fn init() -> NestResult<()> {
    (|| -> Result<(), NestError> {
        unsafe {
            let m = board::info();

            if VirtAddr::from_raw(m.dram.base + m.dram.size) > mode::upper() {
                return Err(NestError::DramOverlap);
            }

            let kernel_space = SpaceBuilder::kernel().build()?;
            let memory = &kernel_space.memory;

            let text_start = (&raw const _kernel_base).addr();
            let text_end = (&raw const _text_end).addr();
            let rodata_start = (&raw const _rodata_start).addr();
            let image_end = kernel_edge();
            let ram_end = m
                .dram
                .base
                .checked_add(m.dram.size)
                .ok_or(NestError::NoRegion)?;
            if !(m.dram.base <= text_start
                && text_start < text_end
                && text_end <= rodata_start
                && rodata_start <= image_end
                && image_end <= ram_end)
            {
                return Err(NestError::NoRegion);
            }
            let ram_flags =
                PteFlags::V | PteFlags::R | PteFlags::W | PteFlags::A | PteFlags::D | PteFlags::G;
            let ro_flags = PteFlags::V | PteFlags::R | PteFlags::A | PteFlags::G;
            let text_flags = ro_flags | PteFlags::X;
            let arch = |flags: PteFlags| flags & (PteFlags::U | PteFlags::G | PteFlags::A | PteFlags::D);

            // 低 identity 是 `Normal` 里的一段登记：整段 DRAM 先占，再按段切分。
            let identity = memory.reserve(
                RegionId::Normal,
                VirtAddr::from_raw(m.dram.base),
                m.dram.size,
            )?;
            kernel_space
                .memory
                .advance_region(RegionId::Normal, image_end)?;
            let mut rest = Some(identity);
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
                let size = end - start;
                let reservation = match rest.take() {
                    Some(whole) if whole.size() == size => whole,
                    Some(whole) => {
                        let (piece, tail) = memory.split_reservation(whole, size)?;
                        rest = Some(tail);
                        piece
                    }
                    None => return Err(NestError::NoRegion),
                };
                let access = Access::from_pte(flags);
                let attrs = arch(flags);
                let region = BorrowedRegion::new(PhysAddr::from_raw(start), size)?;
                memory.attach_static(
                    reservation,
                    region.clone(),
                    0,
                    size,
                    access,
                    attrs,
                )?;
                memory.map_boot_borrowed(
                    VirtAddr::wrap(mode::lower().as_usize() + start),
                    region,
                    0,
                    size,
                    access,
                    attrs,
                )?;
            }

            let tramp_flags =
                PteFlags::V | PteFlags::R | PteFlags::X | PteFlags::A | PteFlags::D | PteFlags::G;
            let trampoline = BorrowedRegion::new(trampoline_pa(), PAGE_SIZE)?;
            memory.map_boot_borrowed(
                TRAMPOLINE,
                trampoline,
                0,
                PAGE_SIZE,
                Access::from_pte(tramp_flags),
                arch(tramp_flags),
            )?;

            let n = hart::hart_count();
            for h in 0..n {
                let frame = crate::tag!(HartFrame, Frame::allocate()?);
                let mut frames = Vec::new();
                frames
                    .try_reserve_exact(1)
                    .map_err(|_| NestError::OutOfMemory)?;
                frames.push(frame);
                let flags = PteFlags::V | PteFlags::R | PteFlags::W | PteFlags::A | PteFlags::D;
                memory.map_boot_frames(
                    HART_FRAME_BASE + h * PAGE_SIZE,
                    frames,
                    Access::from_pte(flags),
                    arch(flags),
                )?;
            }

            let kernel_space = Arc::try_new(kernel_space).map_err(|_| NestError::OutOfMemory)?;
            team::init_kernel(kernel_space.clone())?;
            hart::mark_online();
            satp::set(
                mode::mode(),
                kernel_space.memory.asid().get(),
                kernel_space.memory.root_ppn(),
            );

            identity::flush(kernel_space.memory.asid());
            #[cfg(debug_assertions)]
            crate::machine::layout::validate();

            Ok(())
        }
    })()
    .annotate("initializing unit (kernel space + team)")
}
