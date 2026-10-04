use alloc::boxed::Box;
use alloc::vec::Vec;

use crate::layout::{TEAM_FRAME_BASE, TEAM_FRAME_WINDOW_SIZE};
use crate::memory::PAGE_SIZE;
use crate::memory::manager::MapError;
use crate::memory::manager::addr::{PhysAddr, VirtAddr};
use crate::memory::manager::entry::PteFlags;
use crate::memory::manager::mode;
use crate::memory::manager::table::{Frame, TableNode};

use super::SegmentKind;
use super::index::Index;
use super::map::{Map, Origin, Pending};
use super::salvage::Salvage;

pub(crate) struct SpaceInner {
    pub(crate) root: TableNode,
    pub(crate) user: Option<super::segment::Segment>,
    pub(crate) kernel: super::segment::Segment,
    pub(super) maps: Index,
}

impl core::fmt::Debug for SpaceInner {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("SpaceInner")
            .field("user_attached", &self.user.is_some())
            .field("maps", &self.maps.len())
            .finish()
    }
}

impl SpaceInner {
    pub(crate) fn durable() -> Result<Self, MapError> {
        Ok(Self {
            root: TableNode::root()?,
            user: None,
            kernel: super::segment::Segment::new(
                TEAM_FRAME_BASE.as_usize(),
                TEAM_FRAME_BASE.as_usize() + TEAM_FRAME_WINDOW_SIZE,
            ),
            maps: Index::new(),
        })
    }

    pub(crate) fn dynamic(&mut self, base: usize) {
        assert!(self.user.is_none(), "Space: user segment double attach");
        let edge = mode::upper().as_usize();
        assert!(
            base.is_multiple_of(PAGE_SIZE) && base <= edge,
            "Space: bad user segment [{base:#x}, {edge:#x})"
        );
        self.user = Some(super::segment::Segment::new(base, edge));
    }

    pub(crate) fn allocate(
        &mut self,
        seg: SegmentKind,
        addr: usize,
        size: usize,
    ) -> Result<(), MapError> {
        if size == 0 || !addr.is_multiple_of(PAGE_SIZE) || !size.is_multiple_of(PAGE_SIZE) {
            return Err(MapError::NotAligned);
        }
        let segment = match seg {
            SegmentKind::Normal => self.user.as_mut().ok_or(MapError::NoRegion)?,
            SegmentKind::Kernel => &mut self.kernel,
        };
        segment.allocate(addr, size)
    }

    pub(crate) fn deallocate(&mut self, seg: SegmentKind, addr: usize, size: usize) -> bool {
        let seg = match seg {
            SegmentKind::Normal => match self.user.as_mut() {
                Some(u) => u,
                None => return false,
            },
            SegmentKind::Kernel => &mut self.kernel,
        };
        seg.deallocate(addr, size)
    }

    fn register(&mut self, map: Map) -> Result<(), MapError> {
        let map = Box::try_new(map).map_err(|_| MapError::OutOfMemory)?;
        self.maps.insert(map);
        Ok(())
    }

    pub(crate) fn map(
        &mut self,
        va: VirtAddr,
        size: usize,
        flags: PteFlags,
        pending: Option<Pending>,
    ) -> Result<(), MapError> {
        if size == 0 || !va.as_usize().is_multiple_of(PAGE_SIZE) || !size.is_multiple_of(PAGE_SIZE)
        {
            return Err(MapError::NotAligned);
        }
        if self.overlaps(va, size) {
            return Err(MapError::AlreadyMapped);
        }
        self.register(Map::new(va, size, flags, pending))
    }

    pub(crate) fn claim<F>(
        &mut self,
        va: VirtAddr,
        size: usize,
        flags: PteFlags,
        next_frame: F,
    ) -> Result<(), MapError>
    where
        F: FnMut() -> Result<Frame, MapError>,
    {
        if size == 0 || !va.as_usize().is_multiple_of(PAGE_SIZE) {
            return Err(MapError::NotAligned);
        }
        if self.overlaps(va, size) {
            return Err(MapError::AlreadyMapped);
        }
        let pages = size / PAGE_SIZE;
        self.register(Map::new(va, size, flags, None))?;
        self.install(va, pages, flags, MapMode::Claim(va), next_frame)
    }

    pub(crate) fn attach(
        &mut self,
        vaddr: VirtAddr,
        frames: Vec<Frame>,
        flags: PteFlags,
    ) -> Result<(), MapError> {
        let pages = frames.len();
        if pages == 0 || vaddr.offset() != 0 {
            return Err(MapError::NotAligned);
        }
        let size = pages * PAGE_SIZE;
        if self.overlaps(vaddr, size) {
            return Err(MapError::AlreadyMapped);
        }
        self.register(Map::new(vaddr, size, flags, None))?;
        let mut iter = frames.into_iter();
        self.install(vaddr, pages, flags, MapMode::Claim(vaddr), move || {
            Ok(iter.next().expect("attach: frame iter exhausted"))
        })
    }

    pub(crate) fn borrow(
        &mut self,
        vaddr: VirtAddr,
        paddr: PhysAddr,
        size: usize,
        flags: PteFlags,
    ) -> Result<(), MapError> {
        if size == 0 || vaddr.offset() != 0 || !paddr.is_aligned() || size & (PAGE_SIZE - 1) != 0 {
            return Err(MapError::NotAligned);
        }
        if self.overlaps(vaddr, size) {
            return Err(MapError::AlreadyMapped);
        }
        let mut map = Map::new(vaddr, size, flags, None);
        map.origin = Origin::Borrowed;
        if let Err(error) = self.root.map(vaddr, paddr, size, flags) {
            self.root.unmap(vaddr, size)?;
            return Err(error);
        }
        if let Err(error) = self.register(map) {
            self.root.unmap(vaddr, size)?;
            return Err(error);
        }
        Ok(())
    }

    pub(crate) fn backed(
        &mut self,
        va: VirtAddr,
        backing: alloc::sync::Arc<super::Backing>,
        offset: usize,
        size: usize,
        flags: PteFlags,
        ceiling: PteFlags,
    ) -> Result<(), MapError> {
        let pa = backing.address(offset, size)?;
        if va.offset() != 0 || self.overlaps(va, size) {
            return Err(MapError::AlreadyMapped);
        }
        let access = PteFlags::R | PteFlags::W | PteFlags::X;
        if !(ceiling & access).contains(flags & access) {
            return Err(MapError::WidenDenied);
        }
        let mut map = Map::new(va, size, flags, None);
        backing.alias(true);
        if ceiling.contains(PteFlags::W) {
            backing.map_write(true);
        }
        map.origin = Origin::Backed {
            backing,
            offset,
            ceiling: ceiling & access,
            token: None,
            private: false,
            open: false,
        };
        self.register(map)?;
        if let Err(error) = self.root.map(va, pa, size, flags) {
            self.root.unmap(va, size)?;
            self.maps.remove(va);
            return Err(error);
        }
        Ok(())
    }

    pub(crate) fn bind(&mut self, va: VirtAddr, token: env::PieToken) {
        if let Some(map) = self.resolve_mut(va) {
            if let Origin::Backed { token: source, .. } = &mut map.origin {
                *source = Some(token);
            }
        }
    }

    pub(crate) fn mark_open(&mut self, va: VirtAddr) {
        if let Some(map) = self.resolve_mut(va) {
            if let Origin::Backed { open, .. } = &mut map.origin {
                *open = true;
            }
        }
    }

    pub(crate) fn limit(&mut self, va: VirtAddr, ceiling: PteFlags) {
        if let Some(map) = self.resolve_mut(va) {
            map.origin = Origin::Limited { ceiling };
        }
    }

    pub(crate) fn private(&mut self, va: VirtAddr) {
        if let Some(map) = self.resolve_mut(va) {
            if let Origin::Backed {
                private, backing, ..
            } = &mut map.origin
            {
                if !*private {
                    backing.alias(false);
                    *private = true;
                }
            }
        }
    }

    pub(crate) fn narrow_token(
        &mut self,
        token: env::PieToken,
        access: PteFlags,
    ) -> Result<(), MapError> {
        let mut fault = None;
        let root = &mut self.root;
        self.maps.visit_mut(0, usize::MAX, |map| {
            if fault.is_some() {
                return;
            }
            if let Origin::Backed {
                backing,
                ceiling,
                token: Some(source),
                ..
            } = &mut map.origin
            {
                if *source != token {
                    return;
                }
                let narrowed = *ceiling & access;
                let flags = (map.flags - (PteFlags::R | PteFlags::W | PteFlags::X))
                    | (map.flags & narrowed);
                if let Err(error) = root.protect(map.va, map.size.get(), flags) {
                    fault = Some(error);
                    return;
                }
                if ceiling.contains(PteFlags::W) && !narrowed.contains(PteFlags::W) {
                    backing.map_write(false);
                }
                *ceiling = narrowed;
                map.flags = flags;
            }
        });
        fault.map_or(Ok(()), Err)
    }

    pub(crate) fn unmap(
        &mut self,
        va: VirtAddr,
        size: usize,
        salvage: &mut Salvage,
    ) -> Result<(), MapError> {
        if size == 0 {
            return Ok(());
        }
        let lo = va.as_usize();
        let last = lo.saturating_add(size - 1);

        let mut splits: Vec<Split> = Vec::new();
        for m in self.maps.overlapping(lo, last) {
            let Some((lo_pg, hi_pg)) = intersect(m, lo, last) else {
                continue;
            };
            let pages = m.size.get() / PAGE_SIZE;
            if lo_pg == 0 && hi_pg == pages {
                continue;
            }
            let hole = {
                let n = m.frames.count_range(lo_pg, hi_pg);
                (n > 0 || m.retains_backing())
                    .then(|| m.part(lo_pg, hi_pg - lo_pg, n))
                    .transpose()?
            };
            let right = if lo_pg != 0 && hi_pg < pages {
                Some(m.part(hi_pg, pages - hi_pg, m.frames.count_range(hi_pg, pages))?)
            } else {
                None
            };
            splits.try_reserve(1).map_err(|_| MapError::OutOfMemory)?;
            splits.push(Split { hole, right });
        }

        self.root.prepare(va, size)?;
        let SpaceInner { root, maps, .. } = self;
        let mut n = 0usize;
        while let Some(m) = maps.first_overlap(lo, last) {
            let m_va = m.va.as_usize();
            let pages = m.size.get() / PAGE_SIZE;
            let (lo_pg, hi_pg) = intersect(m, lo, last).expect("unmap intersection");
            let mut m = maps
                .remove(VirtAddr::wrap(m_va))
                .expect("unmap indexed map");
            m.runs(lo_pg, hi_pg, |rva, rsize| root.unmap_prepared(rva, rsize));
            if lo_pg == 0 && hi_pg == pages {
                salvage.take_map(m);
                continue;
            }
            let split = &mut splits[n];
            n += 1;
            m.carve(
                lo_pg,
                hi_pg,
                split.hole.as_deref_mut(),
                split.right.as_deref_mut(),
            );
            if let Some(hole) = split.hole.take() {
                salvage.take_map(hole);
            }
            if let Some(right) = split.right.take() {
                maps.insert(right);
            }
            maps.insert(m);
        }
        debug_assert_eq!(n, splits.len(), "unmap: 两趟的图数不一致");
        Ok(())
    }

    pub(crate) fn holds(&self, seg: SegmentKind, addr: usize, size: usize) -> bool {
        match seg {
            SegmentKind::Normal => self.user.as_ref().is_some_and(|u| u.holds(addr, size)),
            SegmentKind::Kernel => self.kernel.holds(addr, size),
        }
    }

    pub(super) fn maps_in(
        &self,
        va: VirtAddr,
        size: usize,
        allowed: impl Fn(&Map) -> bool,
    ) -> bool {
        if size == 0 {
            return true;
        }
        let Some(last) = va.as_usize().checked_add(size - 1) else {
            return false;
        };
        let mut at = va.as_usize();
        for map in self.maps.overlapping(at, last) {
            if !map.contains(VirtAddr::wrap(at)) || !allowed(map) {
                return false;
            }
            let end = last.min(map.end());
            if end == last {
                return true;
            }
            at = end + 1;
        }
        false
    }

    pub(crate) fn frame() -> Result<Frame, MapError> {
        let frame: Frame = unsafe {
            Box::try_new_zeroed_in(crate::memory::allocator::frame::allocator())
                .map_err(|_| MapError::OutOfMemory)?
                .assume_init()
        };
        Ok(frame)
    }

    pub(crate) fn materialize(&mut self, va: VirtAddr, size: usize) -> Result<(), MapError> {
        let pages = size.div_ceil(PAGE_SIZE);
        let flags = {
            let m = self.resolve_ref(va).ok_or(MapError::NoRegion)?;
            if m.pending != Some(Pending::Lazy) {
                return Err(MapError::NoRegion);
            }
            m.flags | PteFlags::A | PteFlags::D
        };
        self.install(va, pages, flags, MapMode::Materialize, || {
            Ok(crate::tag!(Lazy, Self::frame()?))
        })
    }

    pub(crate) fn protect(
        &mut self,
        va: VirtAddr,
        size: usize,
        flags: PteFlags,
        permissions_only: bool,
    ) -> Result<(), MapError> {
        if size == 0 {
            return Ok(());
        }
        let flags = if permissions_only {
            flags
        } else {
            flags | PteFlags::V
        };
        let lo = va.as_usize();
        let last = lo.saturating_add(size - 1);
        let span = |m: &Map| {
            let s = m.va.as_usize();
            let first = lo.max(s);
            let last = last.min(m.end());
            (first <= last).then_some((s, first, last))
        };
        let covered: usize = self
            .maps
            .overlapping(lo, last)
            .filter_map(|m| span(m))
            .map(|(_, first, last)| last - first + 1)
            .sum();
        if covered != size {
            return Err(MapError::NoRegion);
        }
        {
            let root = &self.root;
            for m in self.maps.overlapping(lo, last) {
                if let Origin::Limited { ceiling } = &m.origin {
                    if span(m).is_some()
                        && !ceiling.contains(flags & (PteFlags::R | PteFlags::W | PteFlags::X))
                    {
                        return Err(MapError::WidenDenied);
                    }
                }
                if let Origin::Backed { ceiling, .. } = &m.origin {
                    if span(m).is_some()
                        && !ceiling.contains(flags & (PteFlags::R | PteFlags::W | PteFlags::X))
                    {
                        return Err(MapError::WidenDenied);
                    }
                }
                if !m.is_borrowed() {
                    continue;
                }
                let Some((s, lo, hi)) = span(m) else { continue };
                let lo_pg = (lo - s) / PAGE_SIZE;
                let hi_pg = (hi - s) / PAGE_SIZE + 1;
                let mut denied = false;
                m.runs(lo_pg, hi_pg, |rva, rsize| {
                    for i in 0..(rsize / PAGE_SIZE) {
                        let page = rva + i * PAGE_SIZE;
                        if let Ok((_, cur)) = root.walk_ref(page)
                            && flags.bits() & !cur.bits() != 0
                        {
                            denied = true;
                        }
                    }
                });
                if denied {
                    return Err(MapError::WidenDenied);
                }
            }
        }
        // Lazy permissions must follow the requested pages, including future faults.
        let mut splits = [None, None];
        for (index, boundary) in [Some(lo), lo.checked_add(size)].into_iter().enumerate() {
            let Some(boundary) = boundary else { continue };
            if let Some(map) = self.resolve_ref(VirtAddr::wrap(boundary))
                && (map.pending == Some(Pending::Lazy) || map.retains_backing())
                && boundary > map.va.as_usize()
            {
                let first = (boundary - map.va.as_usize()) / PAGE_SIZE;
                let pages = map.size.get() / PAGE_SIZE;
                splits[index] = Some((
                    boundary,
                    map.part(first, pages - first, map.frames.count_range(first, pages))?,
                ));
            }
        }
        self.root.prepare(va, size)?;
        for (boundary, mut right) in splits.into_iter().flatten() {
            let key = self
                .resolve_ref(VirtAddr::wrap(boundary))
                .expect("split mapping")
                .va;
            let mut map = self.maps.remove(key).expect("split indexed map");
            let first = (boundary - map.va.as_usize()) / PAGE_SIZE;
            map.frames.move_tail(first, first, &mut right.frames);
            map.size = core::num::NonZeroUsize::new(first * PAGE_SIZE).expect("split prefix");
            self.maps.insert(map);
            self.maps.insert(right);
        }
        let mut fault: Option<MapError> = None;
        let root = &mut self.root;
        self.maps.visit_mut(lo, last, |m| {
            let Some((s, lo, hi)) = span(m) else { return };
            let lo_pg = (lo - s) / PAGE_SIZE;
            let hi_pg = (hi - s) / PAGE_SIZE + 1;
            m.runs(lo_pg, hi_pg, |rva, rsize| {
                if fault.is_none()
                    && let Err(e) = if permissions_only {
                        root.protect_permissions(rva, rsize, flags)
                    } else {
                        root.protect(rva, rsize, flags)
                    }
                {
                    fault = Some(e);
                }
            });
            if permissions_only {
                m.flags = (m.flags - (PteFlags::R | PteFlags::W | PteFlags::X)) | flags;
            } else if m.pending == Some(Pending::Lazy) || m.retains_backing() {
                m.flags = flags;
            }
        });
        match fault {
            Some(e) => Err(e),
            None => Ok(()),
        }
    }

    pub(crate) fn overlaps(&self, start: VirtAddr, size: usize) -> bool {
        size != 0
            && self
                .maps
                .first_overlap(start.as_usize(), start.as_usize().saturating_add(size - 1))
                .is_some()
    }

    pub(super) fn resolve_ref(&self, vaddr: VirtAddr) -> Option<&Map> {
        self.maps.get(vaddr)
    }

    pub(super) fn resolve_mut(&mut self, vaddr: VirtAddr) -> Option<&mut Map> {
        self.maps.get_mut(vaddr)
    }

    pub(super) fn translate(&self, vaddr: VirtAddr) -> Option<(PhysAddr, PteFlags)> {
        self.root
            .walk_ref(vaddr)
            .map(|x| (x.0 + vaddr.offset(), x.1))
            .ok()
    }

    #[cfg(debug_assertions)]
    pub(crate) fn audit(&self) {
        self.maps.audit();
        for m in self.maps.iter() {
            for (i, f) in m.frames.iter() {
                let va = m.va + i * PAGE_SIZE;
                let expect = page_pa(f);
                match self.translate(va) {
                    Some((pa, _)) if pa == expect => {}
                    other => panic!(
                        "space audit @{:#x}: pte {other:?} != frame {expect:#x} (map {:#x}+{})",
                        va.as_usize(),
                        m.va.as_usize(),
                        m.size.get()
                    ),
                }
            }
        }
        self.root
            .mapped(mode::levels() - 1, 0, &mut |va: VirtAddr| {
                let Some(m) = self.resolve_ref(va) else {
                    panic!(
                        "space audit @{:#x}: leaf pte outside every map",
                        va.as_usize()
                    );
                };
                let idx = (va.as_usize() - m.va.as_usize()) / PAGE_SIZE;
                assert!(
                    m.is_materialized(idx),
                    "space audit @{:#x}: leaf pte on unmaterialized page (map {:#x}+{}, {:?})",
                    va.as_usize(),
                    m.va.as_usize(),
                    m.size.get(),
                    m.pending
                );
            });
    }
}

struct Split {
    hole: Option<Box<Map>>,
    right: Option<Box<Map>>,
}

fn intersect(m: &Map, lo: usize, last: usize) -> Option<(usize, usize)> {
    let s = m.va.as_usize();
    let l = lo.max(s);
    let h = last.min(m.end());
    (l <= h).then(|| ((l - s) / PAGE_SIZE, (h - s) / PAGE_SIZE + 1))
}

#[derive(Clone, Copy)]
enum MapMode {
    Materialize,
    Claim(VirtAddr),
}

struct InstallGuard<'a> {
    inner: &'a mut SpaceInner,
    va: VirtAddr,
    installed: usize,
    committed: bool,
    book: MapMode,
}

impl<'a> InstallGuard<'a> {
    fn new(inner: &'a mut SpaceInner, va: VirtAddr, book: MapMode) -> Self {
        Self {
            inner,
            va,
            installed: 0,
            committed: false,
            book,
        }
    }
    fn mark(&mut self) {
        self.installed += 1;
    }
    fn commit(mut self) {
        self.committed = true;
    }
}

#[cfg(debug_assertions)]
fn page_pa(f: &Frame) -> PhysAddr {
    PhysAddr::from_raw(f.as_ptr() as usize)
}

impl Drop for InstallGuard<'_> {
    fn drop(&mut self) {
        if self.committed {
            return;
        }
        if self.installed == 0 {
            if let MapMode::Claim(base) = self.book {
                self.inner.maps.remove(base);
            }
            return;
        }
        for j in 0..self.installed {
            self.inner
                .root
                .unmap_prepared(self.va + j * PAGE_SIZE, PAGE_SIZE);
        }
        match self.book {
            MapMode::Materialize => {
                for j in 0..self.installed {
                    if let Some(m) = self.inner.resolve_mut(self.va + j * PAGE_SIZE) {
                        m.frames.remove(j);
                    }
                }
            }
            MapMode::Claim(va) => {
                self.inner.maps.remove(va);
            }
        }
    }
}

impl SpaceInner {
    fn install<F>(
        &mut self,
        va: VirtAddr,
        pages: usize,
        flags: PteFlags,
        book: MapMode,
        mut next_frame: F,
    ) -> Result<(), MapError>
    where
        F: FnMut() -> Result<Frame, MapError>,
    {
        if let Err(error) = self
            .resolve_mut(va)
            .expect("map exists")
            .reserve_frames(pages)
        {
            if let MapMode::Claim(base) = book {
                self.maps.remove(base);
            }
            return Err(error);
        }
        let mut guard = InstallGuard::new(self, va, book);
        let result: Result<(), MapError> = (|| {
            for i in 0..pages {
                let m_va = va + i * PAGE_SIZE;
                let page = next_frame()?;
                let pa = PhysAddr::from_raw(page.as_ptr() as usize);
                guard.inner.root.map(m_va, pa, PAGE_SIZE, flags)?;
                let map = guard.inner.resolve_mut(m_va).expect("map exists");
                let idx = (m_va.as_usize() - map.va.as_usize()) / PAGE_SIZE;
                map.frames.insert(idx, page);
                guard.mark();
            }
            Ok(())
        })();
        if result.is_ok() {
            guard.commit();
        }
        result
    }
}
