use crate::memory::PAGE_SIZE;
use crate::memory::manager::MapError;
use crate::memory::manager::addr::VirtAddr;
use crate::memory::manager::entry::PteFlags;

use super::super::map::Pending;
use super::super::salvage::Span;
use super::super::{SegmentKind, Space};

pub(crate) struct ShareWindow;

impl ShareWindow {
    pub(crate) fn mmap(space: &Space, size: usize) -> Result<Span, MapError> {
        if size == 0 || !size.is_multiple_of(PAGE_SIZE) {
            return Err(MapError::NotAligned);
        }
        let flags = space.pte_policy(PteFlags::V | PteFlags::R | PteFlags::W);
        space.with(|inner| {
            let va = inner.allocate(SegmentKind::Normal, size)?;
            if let Err(e) = inner.map(va, size, flags, Some(Pending::Lazy)) {
                inner.deallocate(SegmentKind::Normal, va.as_usize(), size);
                return Err(e);
            }
            Ok(Span::new(SegmentKind::Normal, va, size, None))
        })
    }

    pub(crate) fn mmap_at(space: &Space, addr: VirtAddr, size: usize) -> Result<(), MapError> {
        if addr.offset() != 0 {
            return Err(MapError::NotAligned);
        }
        let flags = space.pte_policy(PteFlags::V | PteFlags::R | PteFlags::W);
        space.with(|inner| {
            if inner.overlaps(addr, size) {
                return Err(MapError::AlreadyMapped);
            }
            if !inner
                .user
                .as_mut()
                .is_some_and(|user| user.reserve(addr.as_usize(), size))
            {
                return Err(MapError::NoRegion);
            }
            if let Err(error) = inner.map(addr, size, flags, Some(Pending::Lazy)) {
                inner.deallocate(SegmentKind::Normal, addr.as_usize(), size);
                return Err(error);
            }
            Ok(())
        })
    }

    pub(crate) fn munmap(space: &Space, addr: VirtAddr, size: usize) -> bool {
        {
            let this = &space;
            let seg = SegmentKind::Normal;
            if !this.with_flush(|inner| inner.holds(seg, addr.as_usize(), size)) {
                return false;
            }
            this.release_if(Span::new(seg, addr, size, None), |inner| {
                inner
                    .maps
                    .iter()
                    .filter(|map| {
                        addr.as_usize() < map.va.as_usize().saturating_add(map.size.get())
                            && map.va.as_usize() < addr.as_usize() + size
                    })
                    .all(|map| map.pending == Some(Pending::Lazy))
            })
            .is_ok()
        }
    }
}
