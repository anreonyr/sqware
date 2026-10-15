use alloc::sync::Arc;
use alloc::vec::Vec;
use env::ledger::capsule::{Capsule, PAGE, Region};

use super::space::{Space, SpaceBuilder};
use super::team::{Team, TeamBuilder};
use crate::core::memory::PAGE_SIZE;
use crate::core::memory::address::{PageCount, PageIndex, PhysAddr, VirtAddr};
use crate::core::memory::flat::{Allocation, BorrowedRegion, Flat, FlatRange, Frame};
use crate::core::memory::nest::table::PteFlags;
use crate::core::memory::nest::{Access, Nest, NestError, Placement, RegionId, Reservation};

pub(crate) fn assemble(bytes: &'static [u8]) -> Result<Arc<Team>, NestError> {
    let capsule = Capsule::parse(bytes).ok_or(NestError::NoRegion)?;
    if !(bytes.as_ptr() as usize).is_multiple_of(PAGE) {
        return Err(NestError::NotAligned);
    }
    let mut image_end = crate::machine::layout::IMAGE_BASE.as_usize();
    for i in 0..capsule.count {
        let region = capsule.region(i).ok_or(NestError::NoRegion)?;
        let size = region.pages.checked_mul(PAGE).ok_or(NestError::NoRegion)?;
        if !Space::user_range(VirtAddr::wrap(region.va), size) {
            return Err(NestError::NoRegion);
        }
        image_end = image_end.max(region.va.checked_add(size).ok_or(NestError::NoRegion)?);
    }
    let space = SpaceBuilder::supervisor().build()?;
    let nest = &space.nest;
    for i in 0..capsule.count {
        let region = capsule.region(i).ok_or(NestError::NoRegion)?;
        let size = region.pages.checked_mul(PAGE).ok_or(NestError::NoRegion)?;
        let va = VirtAddr::wrap(region.va);
        let access =
            Access::from_pte(PteFlags::from_bits(region.flags).ok_or(NestError::WidenDenied)?);
        let data_size = region
            .data_pages
            .checked_mul(PAGE)
            .ok_or(NestError::NoRegion)?;
        if data_size > 0 {
            // 来源先备齐：可写段拷进新帧，只读段借用镜像自身那一段物理区间。
            let source = if access.contains(Access::WRITE) {
                image_frames(bytes, &region, data_size)?
            } else {
                let physical = bytes
                    .as_ptr()
                    .addr()
                    .checked_add(region.payload)
                    .ok_or(NestError::NoRegion)?;
                let borrowed =
                    unsafe { BorrowedRegion::new(PhysAddr::from_raw(physical), data_size)? };
                Flat::from_region(borrowed)?.whole()
            };
            let reservation = reserve_at(nest, va, data_size)?;
            nest.attach(reservation, source, access, access)?;
        }
        if size > data_size {
            let bss_start = va + data_size;
            let bss_size = size - data_size;
            let source = Flat::acquire(Allocation::new(bss_size / PAGE_SIZE, PAGE_SIZE)?)?.whole();
            let reservation = reserve_at(nest, bss_start, bss_size)?;
            nest.attach(reservation, source, access, access)?;
        }
    }
    nest.advance_region(RegionId::Normal, image_end)?;
    crate::core::memory::nest::identity::sync_instructions()?;
    let team = TeamBuilder::new(space).spawn()?;
    team.set_default_entry(capsule.entry);
    Ok(team)
}

/// 把镜像里的数据页拷进新帧，返回覆盖整段数据区的来源。
///
/// 帧表与来源都在占用之前备好；失败时调用方的镜像字节不动。
fn image_frames(
    bytes: &'static [u8],
    region: &Region,
    data_size: usize,
) -> Result<FlatRange, NestError> {
    let pages = data_size / PAGE_SIZE;
    let mut indexed = Vec::new();
    indexed
        .try_reserve_exact(pages)
        .map_err(|_| NestError::OutOfMemory)?;
    for page in 0..pages {
        let frame = crate::tag!(Image, Frame::allocate()?);
        let start = region.payload + page * PAGE;
        unsafe {
            core::ptr::copy_nonoverlapping(
                bytes[start..start + PAGE].as_ptr(),
                frame.as_mut_ptr(),
                PAGE,
            );
        }
        indexed.push((PageIndex::new(page), frame));
    }
    let count = PageCount::new(indexed.len()).ok_or(NestError::NotAligned)?;
    Ok(Flat::from_frames(count, &indexed)?.whole())
}

/// 在 `Normal` 窗口内登记一笔固定跨度；不动页表。
fn reserve_at(nest: &Arc<Nest>, start: VirtAddr, size: usize) -> Result<Reservation, NestError> {
    if size == 0 || start.offset() != 0 || !size.is_multiple_of(PAGE_SIZE) {
        return Err(NestError::NotAligned);
    }
    if !nest.window_contains(RegionId::Normal, start, size) {
        return Err(NestError::NoRegion);
    }
    nest.reserve(Placement::exact(RegionId::Normal, start, size)?)
}
