use crate::layout::TASK_STACK_GUARD;
use crate::memory::PAGE_SIZE;
use crate::memory::manager::MapError;
use crate::memory::manager::addr::VirtAddr;
use crate::memory::manager::entry::PteFlags;

use super::super::inner::SpaceInner;
use super::super::map::Pending;
use super::super::salvage::Span;
use super::super::{SegmentKind, Space};

pub(crate) struct StackWindow;

impl StackWindow {
    pub(crate) fn locate(inner: &SpaceInner, size: usize) -> Result<VirtAddr, MapError> {
        if size == 0 || !size.is_multiple_of(PAGE_SIZE) { return Err(MapError::NotAligned); }
        let segment = inner.user.as_ref().ok_or(MapError::NoRegion)?;
        let base = segment.gaps().rev().find_map(|(start, end)| {
            end.checked_sub(size).filter(|&base| base >= start)
        }).ok_or(MapError::OutOfMemory)?;
        Ok(VirtAddr::wrap(base))
    }

    pub(crate) fn claim(space: &Space, size: usize) -> Result<Span, MapError> {
        let slot_size = size
            .checked_add(TASK_STACK_GUARD)
            .ok_or(MapError::NoRegion)?;
        space.with_flush(|inner| {
            let slot_va = Self::locate(inner, slot_size)?;
            inner.allocate(SegmentKind::Normal, slot_va.as_usize(), slot_size)?;
            let guard_flags = space.pte_policy(PteFlags::V | PteFlags::R | PteFlags::W);
            if let Err(e) = inner.map(slot_va, TASK_STACK_GUARD, guard_flags, Some(Pending::Guard))
            {
                inner.deallocate(SegmentKind::Normal, slot_va.as_usize(), slot_size);
                return Err(e);
            }
            let body_flags = space
                .pte_policy(PteFlags::V | PteFlags::R | PteFlags::W | PteFlags::A | PteFlags::D);
            let body_va = slot_va + TASK_STACK_GUARD;
            let next = || Ok(crate::tag!(Stack, SpaceInner::frame()?));
            if let Err(e) = inner.claim(body_va, size, body_flags, next) {
                inner.maps.remove(slot_va);
                inner.deallocate(SegmentKind::Normal, slot_va.as_usize(), slot_size);
                return Err(e);
            }
            Ok(Span::new(SegmentKind::Normal, slot_va, slot_size, None))
        })
    }
}
