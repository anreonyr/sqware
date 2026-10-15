use alloc::sync::Arc;
use alloc::vec::Vec;

use erra::ResultExt;

use riscv::register::satp;

use crate::core::memory::PAGE_SIZE;
use crate::core::memory::address::{PageCount, PageIndex, PhysAddr, VirtAddr};
use crate::core::memory::flat::{BorrowedRegion, Flat, FlatRange, Frame};
use crate::core::memory::nest::table::PteFlags;
use crate::core::memory::nest::{Access, ActiveRange, NestError, Placement, RegionId, identity};
use crate::core::work::unit::space::SpaceBuilder;
use crate::core::work::unit::team;
use crate::machine::board;
use crate::machine::hart;
use crate::machine::image::kernel_edge;
use crate::machine::layout::{HART_FRAME_BASE, TRAMPOLINE, trampoline_pa};
use crate::machine::mode;

use super::attach_exact;

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
            let nest = &kernel_space.nest;

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
            let arch =
                |flags: PteFlags| flags & (PteFlags::U | PteFlags::G | PteFlags::A | PteFlags::D);

            /// 五段低 identity 布局的局部数据：借用来源与两处活动形状都已备好。
            struct Segment {
                size: usize,
                source: FlatRange,
                attrs: PteFlags,
                grid: ActiveRange,
                lower: ActiveRange,
            }

            // 这五段的来源、活动形状与借用凭证都在整段 identity 登记之前备齐：
            // 此后的切分与挂载不再有“来源还没准备好”的失败。
            let mut segments = Vec::new();
            segments
                .try_reserve_exact(5)
                .map_err(|_| NestError::OutOfMemory)?;
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
                let access = Access::from_pte(flags);
                let region = BorrowedRegion::new(PhysAddr::from_raw(start), size)?;
                let source = Flat::from_region(region)?.whole();
                segments.push(Segment {
                    size,
                    source,
                    attrs: arch(flags),
                    grid: ActiveRange::new(VirtAddr::from_raw(start), size, access, access)?,
                    lower: ActiveRange::new(
                        VirtAddr::wrap(mode::lower().as_usize() + start),
                        size,
                        access,
                        access,
                    )?,
                });
            }

            // 整段 DRAM 先占，再抬高窗口下界（抬高之后整段就落在窗外了），失败显式交还。
            let dram = VirtAddr::from_raw(m.dram.base);
            if !nest.window_contains(RegionId::Normal, dram, m.dram.size) {
                return Err(NestError::NoRegion);
            }
            let identity = nest.reserve(Placement::exact(RegionId::Normal, dram, m.dram.size)?)?;
            if let Err(error) = nest.advance_region(RegionId::Normal, image_end) {
                let _ = nest.release_reservation(identity);
                return Err(error);
            }

            // 切分与挂载：剩下未附着的 tail 由本处交还；已交给 Nest 的 piece 按 Nest 责任，不双放。
            let mut rest = Some(identity);
            let mut failed = None;
            for segment in segments {
                let reservation = match rest.take() {
                    Some(whole) if whole.size() == segment.size => whole,
                    Some(whole) => match nest.split_reservation(whole, segment.size) {
                        Ok((piece, tail)) => {
                            rest = Some(tail);
                            piece
                        }
                        Err(error) => {
                            failed = Some(error);
                            break;
                        }
                    },
                    None => {
                        failed = Some(NestError::NoRegion);
                        break;
                    }
                };
                let grid = nest.attach_shaped(
                    reservation,
                    segment.source.clone(),
                    segment.attrs,
                    &[segment.grid],
                );
                if let Err(error) = grid {
                    failed = Some(error);
                    break;
                }
                let lower = nest.reserve(Placement::exact(
                    RegionId::Static,
                    segment.lower.start,
                    segment.size,
                )?);
                let lower = match lower {
                    Ok(reservation) => reservation,
                    Err(error) => {
                        failed = Some(error);
                        break;
                    }
                };
                if let Err(error) =
                    nest.attach_shaped(lower, segment.source, segment.attrs, &[segment.lower])
                {
                    failed = Some(error);
                    break;
                }
            }
            if let Some(error) = failed {
                if let Some(tail) = rest.take() {
                    let _ = nest.release_reservation(tail);
                }
                return Err(error);
            }

            let tramp_flags =
                PteFlags::V | PteFlags::R | PteFlags::X | PteFlags::A | PteFlags::D | PteFlags::G;
            let trampoline = BorrowedRegion::new(trampoline_pa(), PAGE_SIZE)?;
            let source = Flat::from_region(trampoline)?.whole();
            attach_exact(
                nest,
                TRAMPOLINE,
                source,
                Access::from_pte(tramp_flags),
                arch(tramp_flags),
            )?;

            let n = hart::hart_count();
            for h in 0..n {
                let frame = crate::tag!(HartFrame, Frame::allocate()?);
                let mut indexed = Vec::new();
                indexed
                    .try_reserve_exact(1)
                    .map_err(|_| NestError::OutOfMemory)?;
                indexed.push((PageIndex::ZERO, frame));
                let source =
                    Flat::from_frames(PageCount::new(1).expect("one frame"), &indexed)?.whole();
                let flags = PteFlags::V | PteFlags::R | PteFlags::W | PteFlags::A | PteFlags::D;
                attach_exact(
                    nest,
                    HART_FRAME_BASE + h * PAGE_SIZE,
                    source,
                    Access::from_pte(flags),
                    arch(flags),
                )?;
            }

            let kernel_space = Arc::try_new(kernel_space).map_err(|_| NestError::OutOfMemory)?;
            team::init_kernel(kernel_space.clone())?;
            hart::mark_online();
            satp::set(
                mode::mode(),
                kernel_space.nest.asid().get(),
                kernel_space.nest.root_ppn(),
            );

            identity::flush(kernel_space.nest.asid());
            #[cfg(debug_assertions)]
            crate::machine::layout::validate();

            Ok(())
        }
    })()
    .annotate("initializing unit (kernel space + team)")
}
