use crate::memory::manager::MapError;
use crate::memory::manager::addr::VirtAddr;
use crate::memory::manager::entry::PteFlags;

use super::super::inner::SpaceInner;
use super::super::salvage::Span;
use super::super::{SegmentKind, Space};

pub(crate) struct HeapWindow;

impl HeapWindow {
    pub(crate) fn allocate(space: &Space, size: usize) -> Result<Span, MapError> {
        let flags =
            space.pte_policy(PteFlags::V | PteFlags::R | PteFlags::W | PteFlags::A | PteFlags::D);
        space.with_flush(|inner| {
            let va = inner.allocate(SegmentKind::Normal, size)?;
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
            this.release(Span::new(seg, addr, size, None)).is_ok()
        }
    }
}