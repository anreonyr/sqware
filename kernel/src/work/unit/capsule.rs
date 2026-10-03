use alloc::sync::Arc;
use env::ledger::capsule::{Capsule, PAGE};

use super::space::{Backing, Pending, Space, SpaceBuilder};
use super::team::{Team, TeamBuilder};
use crate::memory::manager::{MapError, addr::VirtAddr, entry::PteFlags};

pub(crate) fn assemble(bytes: &'static [u8]) -> Result<Arc<Team>, MapError> {
    let capsule = Capsule::parse(bytes).ok_or(MapError::NoRegion)?;
    if !(bytes.as_ptr() as usize).is_multiple_of(PAGE) {
        return Err(MapError::NotAligned);
    }
    for i in 0..capsule.count {
        let region = capsule.region(i).ok_or(MapError::NoRegion)?;
        if !Space::user_range(region.va, region.pages * PAGE) {
            return Err(MapError::NoRegion);
        }
    }
    let space = SpaceBuilder::supervisor().build()?;
    space.with_flush(|inner| {
        inner.dynamic(PAGE);
        for i in 0..capsule.count {
            let region = capsule.region(i).ok_or(MapError::NoRegion)?;
            let size = region.pages * PAGE;
            if inner.overlaps(VirtAddr::wrap(region.va), size)
                || !inner
                    .user
                    .as_mut()
                    .is_some_and(|segment| segment.reserve(region.va, size))
            {
                return Err(MapError::AlreadyMapped);
            }
            let va = VirtAddr::wrap(region.va);
            let access = PteFlags::from_bits(region.flags).ok_or(MapError::WidenDenied)?;
            let flags = space.pte_policy(access | PteFlags::V | PteFlags::A | PteFlags::D);
            let data_size = region.data_pages * PAGE;
            if data_size > 0 {
                if access.contains(PteFlags::W) {
                    let mut frames = alloc::vec::Vec::new();
                    frames
                        .try_reserve(region.data_pages)
                        .map_err(|_| MapError::OutOfMemory)?;
                    for page in 0..region.data_pages {
                        let mut frame = crate::tag!(Image, super::space::inner_frame()?);
                        let start = region.payload + page * PAGE;
                        frame.copy_from_slice(&bytes[start..start + PAGE]);
                        frames.push(frame);
                    }
                    inner.attach(va, frames, flags)?;
                    inner.limit(va, access);
                } else {
                    let backing =
                        Backing::region(bytes.as_ptr() as usize + region.payload, data_size)?;
                    inner.backed(va, backing, 0, data_size, flags, access)?;
                }
            }
            if size > data_size {
                inner.map(va + data_size, size - data_size, flags, Some(Pending::Lazy))?;
                inner.limit(va + data_size, access);
            }
        }
        Ok::<_, MapError>(())
    })?;
    super::space::sync_instructions()?;
    let team = TeamBuilder::new(space).spawn()?;
    team.set_default_entry(capsule.entry);
    Ok(team)
}
