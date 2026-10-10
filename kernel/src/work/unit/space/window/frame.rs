use super::super::inner::SpaceInner;
use super::super::salvage::Span;
use super::super::{SegmentKind, Space};
use crate::memory::PAGE_SIZE;
use crate::memory::manager::MapError;
use crate::memory::manager::addr::VirtAddr;
use crate::memory::manager::entry::PteFlags;

pub(crate) struct FrameWindow;

impl FrameWindow {
    pub(crate) fn claim(space: &Space) -> Result<Span, MapError> {
        space.with_flush(|inner| {
            let base = inner
                .kernel
                .gaps()
                .find_map(|(start, end)| (end - start >= PAGE_SIZE).then_some(start))
                .ok_or(MapError::OutOfMemory)?;
            inner.allocate(SegmentKind::Kernel, base, PAGE_SIZE)?;
            let va = VirtAddr::wrap(base);
            let flags = PteFlags::V | PteFlags::R | PteFlags::W | PteFlags::A | PteFlags::D;
            let next = || Ok(crate::tag!(Trap, SpaceInner::frame()?));
            if let Err(e) = inner.claim(va, PAGE_SIZE, flags, next) {
                inner.deallocate(SegmentKind::Kernel, va.as_usize(), PAGE_SIZE);
                return Err(e);
            }
            let pa = inner.translate(va).expect("frame claimed").0;
            Ok(Span::new(SegmentKind::Kernel, va, PAGE_SIZE, Some(pa)))
        })
    }
}
