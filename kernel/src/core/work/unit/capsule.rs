use alloc::sync::Arc;
use alloc::vec::Vec;
use env::ledger::capsule::{Capsule, PAGE};

use super::space::SpaceBuilder;
use super::team::{Team, TeamBuilder};
use crate::core::memory::address::{PhysAddr, VirtAddr};
use crate::core::memory::flat::Frame;
use crate::core::memory::nest::table::PteFlags;
use crate::core::memory::nest::{Access, NestError, RegionId};
use crate::core::work::unit::space::Memory;

pub(crate) fn assemble(bytes: &'static [u8]) -> Result<Arc<Team>, NestError> {
    let capsule = Capsule::parse(bytes).ok_or(NestError::NoRegion)?;
    if !(bytes.as_ptr() as usize).is_multiple_of(PAGE) {
        return Err(NestError::NotAligned);
    }
    let mut image_end = crate::machine::layout::IMAGE_BASE.as_usize();
    for i in 0..capsule.count {
        let region = capsule.region(i).ok_or(NestError::NoRegion)?;
        let size = region.pages.checked_mul(PAGE).ok_or(NestError::NoRegion)?;
        if !Memory::user_range(region.va, size) {
            return Err(NestError::NoRegion);
        }
        image_end = image_end.max(region.va.checked_add(size).ok_or(NestError::NoRegion)?);
    }
    let space = SpaceBuilder::supervisor().build()?;
    for i in 0..capsule.count {
        let region = capsule.region(i).ok_or(NestError::NoRegion)?;
        let size = region.pages.checked_mul(PAGE).ok_or(NestError::NoRegion)?;
        let va = VirtAddr::wrap(region.va);
        let access = Access::from_pte(
            PteFlags::from_bits(region.flags).ok_or(NestError::WidenDenied)?,
        );
        let data_size = region
            .data_pages
            .checked_mul(PAGE)
            .ok_or(NestError::NoRegion)?;
        if data_size > 0 {
            let data_reservation = space.memory.reserve(RegionId::Normal, va, data_size)?;
            if access.contains(Access::WRITE) {
                let mut frames = Vec::new();
                frames
                    .try_reserve_exact(region.data_pages)
                    .map_err(|_| NestError::OutOfMemory)?;
                for page in 0..region.data_pages {
                    let frame = crate::tag!(Image, Frame::allocate()?);
                    let start = region.payload + page * PAGE;
                    unsafe {
                        core::ptr::copy_nonoverlapping(
                            bytes[start..start + PAGE].as_ptr(),
                            frame.as_mut_ptr(),
                            PAGE,
                        );
                    }
                    frames.push(frame);
                }
                space
                    .memory
                    .map_frames(data_reservation, frames, access, access)?;
            } else {
                let physical = bytes
                    .as_ptr()
                    .addr()
                    .checked_add(region.payload)
                    .ok_or(NestError::NoRegion)?;
                unsafe {
                    space.memory.map_borrowed(
                        data_reservation,
                        PhysAddr::from_raw(physical),
                        access,
                        access,
                    )?;
                }
            }
        }
        if size > data_size {
            let bss_start = va + data_size;
            let bss_size = size - data_size;
            let reservation = space.memory.reserve(RegionId::Normal, bss_start, bss_size)?;
            space.memory.map_lazy(reservation, access, access)?;
        }
    }
    space.memory.advance_region(RegionId::Normal, image_end)?;
    crate::core::memory::nest::identity::sync_instructions()?;
    let team = TeamBuilder::new(space).spawn()?;
    team.set_default_entry(capsule.entry);
    Ok(team)
}
