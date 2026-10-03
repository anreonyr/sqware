use alloc::boxed::Box;
use alloc::sync::Arc;
use alloc::vec::Vec;
use core::num::NonZeroUsize;

use super::Backing;
use crate::memory::PAGE_SIZE;
use crate::memory::manager::MapError;
use crate::memory::manager::addr::VirtAddr;
use crate::memory::manager::entry::PteFlags;
use crate::memory::manager::table::Frame;

#[derive(Debug, Clone)]
pub(super) enum Origin {
    Owned,
    Borrowed,
    Limited {
        ceiling: PteFlags,
    },
    Backed {
        backing: Arc<Backing>,
        offset: usize,
        ceiling: PteFlags,
        token: Option<env::PieToken>,
        private: bool,
        open: bool,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Pending {
    Lazy,
    Guard,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PendingState {
    Absent,
    Materialized,
    Lazy,
    Guard,
}

#[derive(Debug)]
pub(crate) struct Frames {
    v: Vec<(usize, Frame)>,
}

impl Frames {
    pub(super) const fn new() -> Self {
        Self { v: Vec::new() }
    }

    pub(super) fn reserve(&mut self, add: usize) -> Result<(), MapError> {
        self.v.try_reserve(add).map_err(|_| MapError::OutOfMemory)
    }

    pub(super) fn insert(&mut self, page: usize, frame: Frame) {
        debug_assert!(
            self.v.len() < self.v.capacity(),
            "frames: insert without reserve (page {page})"
        );
        let at = self.v.partition_point(|(k, _)| *k < page);
        assert!(
            self.v.get(at).is_none_or(|(k, _)| *k != page),
            "frames: double insert @page {page}"
        );
        self.v.insert(at, (page, frame));
    }

    pub(super) fn remove(&mut self, page: usize) -> Option<Frame> {
        let at = self.v.partition_point(|(k, _)| *k < page);
        matches!(self.v.get(at), Some((k, _)) if *k == page).then(|| self.v.remove(at).1)
    }

    #[cfg(debug_assertions)]
    pub(super) fn contains(&self, page: usize) -> bool {
        let at = self.v.partition_point(|(k, _)| *k < page);
        matches!(self.v.get(at), Some((k, _)) if *k == page)
    }

    #[cfg(debug_assertions)]
    pub(super) fn iter(&self) -> impl Iterator<Item = (usize, &Frame)> {
        self.v.iter().map(|(k, f)| (*k, f))
    }

    pub(super) fn range(&self, lo: usize, hi: usize) -> impl Iterator<Item = (usize, &Frame)> {
        let from = self.v.partition_point(|(k, _)| *k < lo);
        let to = self.v.partition_point(|(k, _)| *k < hi);
        self.v[from..to].iter().map(|(k, f)| (*k, f))
    }

    pub(super) fn count_range(&self, lo: usize, hi: usize) -> usize {
        self.range(lo, hi).count()
    }

    pub(super) fn move_range(&mut self, lo: usize, hi: usize, shift: usize, to: &mut Frames) {
        let from = self.v.partition_point(|(k, _)| *k < lo);
        let mut left = self.v.partition_point(|(k, _)| *k < hi) - from;
        debug_assert!(
            to.v.capacity() - to.v.len() >= left,
            "frames: move_range without reserve"
        );
        while left > 0 {
            let (k, f) = self.v.remove(from);
            to.v.push((k - shift, f));
            left -= 1;
        }
    }

    pub(super) fn move_tail(&mut self, from: usize, key_sub: usize, to: &mut Frames) {
        let at = self.v.partition_point(|(k, _)| *k < from);
        debug_assert!(
            to.v.capacity() - to.v.len() >= self.v.len() - at,
            "frames: move_tail without reserve"
        );
        while self.v.len() > at {
            let (k, f) = self.v.remove(at);
            to.v.push((k - key_sub, f));
        }
    }

    pub(super) fn shift_keys(&mut self, sub: usize) {
        for (k, _) in self.v.iter_mut() {
            *k -= sub;
        }
    }
}

#[derive(Debug)]
pub(crate) struct Map {
    pub(super) next: Option<Box<Map>>,
    pub(super) va: VirtAddr,
    pub(super) size: NonZeroUsize,
    pub(super) flags: PteFlags,
    pub(super) pending: Option<Pending>,
    pub(super) frames: Frames,
    pub(super) origin: Origin,
}

impl Drop for Map {
    fn drop(&mut self) {
        if let Origin::Backed {
            backing,
            ceiling,
            private,
            ..
        } = &self.origin
        {
            if !private {
                backing.alias(false);
            }
            if ceiling.contains(PteFlags::W) {
                backing.map_write(false);
            }
        }
        let mut cur = self.next.take();
        while let Some(mut node) = cur {
            cur = node.next.take();
        }
    }
}

impl Map {
    pub(super) fn new(
        va: VirtAddr,
        size: usize,
        flags: PteFlags,
        pending: Option<Pending>,
    ) -> Self {
        Self {
            next: None,
            va,
            size: NonZeroUsize::new(size).expect("map size must be non-zero"),
            flags,
            pending,
            frames: Frames::new(),
            origin: Origin::Owned,
        }
    }

    pub(super) fn contains(&self, vaddr: VirtAddr) -> bool {
        vaddr >= self.va && vaddr.as_usize() - self.va.as_usize() < self.size.get()
    }

    #[cfg(debug_assertions)]
    pub(super) fn is_materialized(&self, idx: usize) -> bool {
        match self.pending {
            None => true,
            Some(Pending::Lazy) => self.frames.contains(idx),
            Some(Pending::Guard) => false,
        }
    }

    pub(super) fn is_borrowed(&self) -> bool {
        matches!(
            self.origin,
            Origin::Borrowed
                | Origin::Backed {
                    token: Some(_),
                    private: false,
                    ..
                }
        )
    }

    pub(super) fn retains_backing(&self) -> bool {
        matches!(self.origin, Origin::Backed { .. })
    }

    pub(super) fn reserve_frames(&mut self, pages: usize) -> Result<(), MapError> {
        self.frames.reserve(pages)
    }

    pub(super) fn part(
        &self,
        first_pg: usize,
        pages: usize,
        frames: usize,
    ) -> Result<Box<Map>, MapError> {
        let mut map = Map::new(
            self.va + first_pg * PAGE_SIZE,
            pages * PAGE_SIZE,
            self.flags,
            self.pending,
        );
        map.origin = match &self.origin {
            Origin::Backed {
                backing,
                offset,
                ceiling,
                token,
                private,
                open,
            } => {
                if !private {
                    backing.alias(true);
                }
                if ceiling.contains(PteFlags::W) {
                    backing.map_write(true);
                }
                Origin::Backed {
                    backing: backing.clone(),
                    offset: offset + first_pg * PAGE_SIZE,
                    ceiling: *ceiling,
                    token: *token,
                    private: *private,
                    open: *open,
                }
            }
            other => other.clone(),
        };
        map.reserve_frames(frames)?;
        Box::try_new(map).map_err(|_| MapError::OutOfMemory)
    }

    pub(super) fn runs(&self, lo_pg: usize, hi_pg: usize, mut apply: impl FnMut(VirtAddr, usize)) {
        match self.pending {
            None => apply(self.va + lo_pg * PAGE_SIZE, (hi_pg - lo_pg) * PAGE_SIZE),
            Some(Pending::Lazy) => {
                let mut run: Option<(usize, usize)> = None;
                for (pg, _) in self.frames.range(lo_pg, hi_pg) {
                    run = match run {
                        Some((start, len)) if start + len == pg => Some((start, len + 1)),
                        Some((start, len)) => {
                            apply(self.va + start * PAGE_SIZE, len * PAGE_SIZE);
                            Some((pg, 1))
                        }
                        None => Some((pg, 1)),
                    };
                }
                if let Some((start, len)) = run {
                    apply(self.va + start * PAGE_SIZE, len * PAGE_SIZE);
                }
            }
            Some(Pending::Guard) => {}
        }
    }

    pub(super) fn carve(
        &mut self,
        lo_pg: usize,
        hi_pg: usize,
        hole: Option<&mut Map>,
        right: Option<&mut Map>,
    ) {
        let pages = self.size.get() / PAGE_SIZE;
        debug_assert!(lo_pg < hi_pg && hi_pg <= pages);
        if let Some(hole) = hole {
            self.frames
                .move_range(lo_pg, hi_pg, lo_pg, &mut hole.frames);
        }
        if lo_pg == 0 {
            debug_assert!(right.is_none(), "carve: 洞在头时由本图重绕，无独立右段");
            self.frames.shift_keys(hi_pg);
            self.va += hi_pg * PAGE_SIZE;
            if let Origin::Backed { offset, .. } = &mut self.origin {
                *offset += hi_pg * PAGE_SIZE;
            }
            self.size = NonZeroUsize::new((pages - hi_pg) * PAGE_SIZE).expect("non-zero");
            return;
        }
        self.size = NonZeroUsize::new(lo_pg * PAGE_SIZE).expect("non-zero");
        if let Some(right) = right {
            debug_assert!(hi_pg < pages, "carve: 洞在尾时无独立右段");
            self.frames.move_tail(lo_pg, hi_pg, &mut right.frames);
        }
    }
}
