use crate::memory::PAGE_SIZE;
use crate::memory::manager::MapError;
use crate::memory::manager::addr::VirtAddr;
use crate::memory::manager::entry::PteFlags;

use super::super::inner::SpaceInner;
use super::super::salvage::Span;
use super::super::{SegmentKind, Space};

pub(crate) struct HeapWindow;

impl HeapWindow {
    pub(crate) fn locate(inner: &SpaceInner, size: usize) -> Result<VirtAddr, MapError> {
        if size == 0 || !size.is_multiple_of(PAGE_SIZE) { return Err(MapError::NotAligned); }
        let segment = inner.user.as_ref().ok_or(MapError::NoRegion)?;
        let base = segment.gaps().find_map(|(start, end)|
            (end - start >= size).then_some(start)).ok_or(MapError::OutOfMemory)?;
        Ok(VirtAddr::wrap(base))
    }

    pub(crate) fn allocate(space: &Space, size: usize) -> Result<Span, MapError> {
        let flags =
            space.pte_policy(PteFlags::V | PteFlags::R | PteFlags::W | PteFlags::A | PteFlags::D);
        space.with_flush(|inner| {
            let va = Self::locate(inner, size)?;
            inner.allocate(SegmentKind::Normal, va.as_usize(), size)?;
            let next = || Ok(crate::tag!(Heap, SpaceInner::frame()?));
            if let Err(e) = inner.claim(va, size, flags, next) {
                inner.deallocate(SegmentKind::Normal, va.as_usize(), size);
                return Err(e);
            }
            Ok(Span::new(SegmentKind::Normal, va, size, None))
        })
    }

    pub(crate) fn deallocate(space: &Space, addr: VirtAddr, size: usize) -> bool {
        {
            let this = &space;
            let seg = SegmentKind::Normal;
            if !this.with_flush(|inner| inner.holds(seg, addr.as_usize(), size)) {
                return false;
            }
            this.release_if(Span::new(seg, addr, size, None), |inner| {
                inner.maps_in(addr, size, |map| {
                    map.pending.is_none() && !map.is_borrowed()
                })
            })
            .is_ok()
        }
    }
}
