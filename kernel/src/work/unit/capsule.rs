use alloc::sync::Arc;
use alloc::vec::Vec;
use env::ledger::capsule::{Capsule, PAGE};

use super::space::{RegionId, Space, SpaceBuilder};
use super::team::{Team, TeamBuilder};
use crate::memory::manager::{Frame, MapError, addr::{PhysAddr, VirtAddr}, entry::PteFlags};

pub(crate) fn assemble(bytes: &'static [u8]) -> Result<Arc<Team>, MapError> {
    let capsule = Capsule::parse(bytes).ok_or(MapError::NoRegion)?;
    if !(bytes.as_ptr() as usize).is_multiple_of(PAGE) {
        return Err(MapError::NotAligned);
    }
    let mut image_end = crate::layout::IMAGE_BASE.as_usize();
    for i in 0..capsule.count {
        let region = capsule.region(i).ok_or(MapError::NoRegion)?;
        let size = region.pages.checked_mul(PAGE).ok_or(MapError::NoRegion)?;
        if !Space::user_range(region.va, size) {
            return Err(MapError::NoRegion);
        }
        image_end = image_end.max(region.va.checked_add(size).ok_or(MapError::NoRegion)?);
    }
    let space = SpaceBuilder::supervisor().build()?;
    for i in 0..capsule.count {
        let region = capsule.region(i).ok_or(MapError::NoRegion)?;
        let size = region.pages.checked_mul(PAGE).ok_or(MapError::NoRegion)?;
        let va = VirtAddr::wrap(region.va);
        let access = PteFlags::from_bits(region.flags).ok_or(MapError::WidenDenied)?;
        let current = access | PteFlags::V | PteFlags::A | PteFlags::D;
        let data_size = region
            .data_pages
            .checked_mul(PAGE)
            .ok_or(MapError::NoRegion)?;
        if data_size > 0 {
            let data_reservation = space.reserve(RegionId::Normal, va, data_size)?;
            if access.contains(PteFlags::W) {
                let mut frames = Vec::new();
                frames
                    .try_reserve_exact(region.data_pages)
                    .map_err(|_| MapError::OutOfMemory)?;
                for page in 0..region.data_pages {
                    let frame = crate::tag!(Image, Frame::allocate()?);
                    let start = region.payload + page * PAGE;
                    // SAFETY: each newly allocated frame is not yet mapped.
                    unsafe {
                        core::ptr::copy_nonoverlapping(
                            bytes[start..start + PAGE].as_ptr(),
                            frame.as_mut_ptr(),
                            PAGE,
                        );
                    }
                    frames.push(frame);
                }
                space.map_frames(data_reservation, frames, access, current)?;
            } else {
                let physical = bytes
                    .as_ptr()
                    .addr()
                    .checked_add(region.payload)
                    .ok_or(MapError::NoRegion)?;
                // SAFETY: `bytes` is static, page-aligned, and remains reserved for every map.
                unsafe {
                    space.map_borrowed(
                        data_reservation,
                        PhysAddr::from_raw(physical),
                        access,
                        current,
                    )?;
                }
            }
        }
        if size > data_size {
            let bss_start = va + data_size;
            let bss_size = size - data_size;
            let reservation = space.reserve(RegionId::Normal, bss_start, bss_size)?;
            space.map_lazy(reservation, access, current)?;
        }
    }
    space.advance_region(RegionId::Normal, image_end)?;
    super::space::sync_instructions()?;
    let team = TeamBuilder::new(space).spawn()?;
    team.set_default_entry(capsule.entry);
    Ok(team)
}
