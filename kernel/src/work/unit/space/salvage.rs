use alloc::boxed::Box;
use core::num::NonZeroUsize;

use super::map::Map;
use super::outer::Space;
use super::segment::SegmentKind;
use crate::memory::manager::addr::{PhysAddr, VirtAddr};
use crate::memory::manager::asid::{self, Deaf};

#[derive(Clone, Copy, Debug)]
pub(crate) struct Span {
    pub(crate) seg: SegmentKind,
    pub(crate) va: VirtAddr,
    pub(crate) size: NonZeroUsize,
    pub(crate) pa: Option<PhysAddr>,
}

impl Span {
    pub(crate) fn new(seg: SegmentKind, va: VirtAddr, size: usize, pa: Option<PhysAddr>) -> Self {
        Self {
            seg,
            va,
            size: NonZeroUsize::new(size).expect("span size must be non-zero"),
            pa,
        }
    }
}

#[must_use = "salvage holds frames/segments that must be reclaimed after eviction"]
pub(crate) struct Salvage {
    maps: Option<Box<Map>>,
    span: Option<Span>,
}

impl Salvage {
    pub(crate) const fn new() -> Self {
        Self {
            maps: None,
            span: None,
        }
    }

    pub(super) fn take_map(&mut self, mut map: Box<Map>) {
        debug_assert!(
            map.left.is_none() && map.right.is_none(),
            "salvage: indexed map"
        );
        map.left = self.maps.take();
        self.maps = Some(map);
    }

    pub(super) fn take_span(&mut self, span: Span) {
        debug_assert!(self.span.is_none(), "salvage: 一次拆除至多一条 Span");
        self.span = Some(span);
    }

    fn chain_len(&self) -> usize {
        let mut n = 0;
        let mut cur = self.maps.as_deref();
        while let Some(m) = cur {
            n += 1;
            cur = m.left.as_deref();
        }
        n
    }

    pub(crate) fn reclaim(mut self, space: &Space) -> Result<(), Deaf> {
        let maps = self.maps.take();
        let span = self.span.take();
        if maps.is_none() && span.is_none() {
            return Ok(());
        }
        asid::shootdown(space.asid())?;
        space.with(|inner| {
            if let Some(span) = span {
                let ok = inner.deallocate(span.seg, span.va.as_usize(), span.size.get());
                debug_assert!(ok, "salvage: segment mismatch on reclaim {:?}", span.va);
            }
        });
        drop(maps);
        Ok(())
    }
}

impl Drop for Salvage {
    fn drop(&mut self) {
        debug_assert!(
            self.maps.is_none() && self.span.is_none(),
            "salvage dropped unreclaimed: {} maps, {} spans",
            self.chain_len(),
            usize::from(self.span.is_some())
        );
    }
}
