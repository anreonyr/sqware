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
use super::map::{Map, Origin, Pending};
use super::salvage::Salvage;

pub(crate) struct SpaceInner {
    pub(crate) root: TableNode,
    pub(crate) user: Option<super::segment::Segment>,
    pub(crate) kernel: super::segment::Segment,
    #[allow(clippy::vec_box)]
    pub(crate) maps: Vec<Box<Map>>,
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
            maps: Vec::new(),
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

    pub(crate) fn allocate(&mut self, seg: SegmentKind, addr: usize, size: usize) -> Result<(), MapError> {
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
        self.maps
            .try_reserve(1)
            .map_err(|_| MapError::OutOfMemory)?;
        let map = Box::try_new(map).map_err(|_| MapError::OutOfMemory)?;
        self.maps.push(map);
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
            self.root.unmap(vaddr, size);
            return Err(error);
        }
        if let Err(error) = self.register(map) {
            self.root.unmap(vaddr, size);
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
        if ceiling.contains(PteFlags::W) { backing.map_write(true); }
        map.origin = Origin::Backed { backing, offset, ceiling: ceiling & access, token: None, private: false, open: false };
        self.register(map)?;
        if let Err(error) = self.root.map(va, pa, size, flags) {
            self.root.unmap(va, size);
            self.maps.retain(|map| map.va != va);
            return Err(error);
        }
        Ok(())
    }

    pub(crate) fn bind(&mut self, va: VirtAddr, token: env::PieToken) {
        if let Some(map) = self.resolve_mut(va) {
            if let Origin::Backed { token: source, .. } = &mut map.origin { *source = Some(token); }
        }
    }

    pub(crate) fn mark_open(&mut self, va: VirtAddr) {
        if let Some(map) = self.resolve_mut(va) {
            if let Origin::Backed { open, .. } = &mut map.origin { *open = true; }
        }
    }

    pub(crate) fn limit(&mut self, va: VirtAddr, ceiling: PteFlags) {
        if let Some(map) = self.resolve_mut(va) { map.origin = Origin::Limited { ceiling }; }
    }

    pub(crate) fn private(&mut self, va: VirtAddr) {
        if let Some(map) = self.resolve_mut(va) {
            if let Origin::Backed { private, backing, .. } = &mut map.origin { if !*private { backing.alias(false); *private = true; } }
        }
    }

    pub(crate) fn narrow_token(&mut self, token: env::PieToken, access: PteFlags) -> Result<(), MapError> {
        for map in &mut self.maps {
            if let Origin::Backed { backing, ceiling, token: Some(source), .. } = &mut map.origin {
                if *source != token { continue; }
                let narrowed = *ceiling & access;
                let flags = (map.flags - (PteFlags::R | PteFlags::W | PteFlags::X)) | (map.flags & narrowed);
                self.root.protect(map.va, map.size.get(), flags)?;
                if ceiling.contains(PteFlags::W) && !narrowed.contains(PteFlags::W) {
                    backing.map_write(false);
                }
                *ceiling = narrowed;
                map.flags = flags;
            }
        }
        Ok(())
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
        let end = lo.saturating_add(size);

        let mut splits: Vec<Split> = Vec::new();
        let mut rights = 0usize;
        for m in self.maps.iter() {
            let Some((lo_pg, hi_pg)) = intersect(m, lo, end) else {
                continue;
            };
            let s = m.va.as_usize();
            if lo <= s && end >= s.saturating_add(m.size.get()) {
                continue;
            }
            let pages = m.size.get() / PAGE_SIZE;
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
            rights += usize::from(right.is_some());
            splits.try_reserve(1).map_err(|_| MapError::OutOfMemory)?;
            splits.push(Split { hole, right });
        }
        self.maps
            .try_reserve(rights)
            .map_err(|_| MapError::OutOfMemory)?;

        let SpaceInner { root, maps, .. } = self;
        let mut i = 0usize;
        let mut n = 0usize;
        while i < maps.len() {
            let m_va = maps[i].va.as_usize();
            let m_size = maps[i].size.get();
            let l = lo.max(m_va);
            let h = end.min(m_va.saturating_add(m_size));
            if l >= h {
                i += 1;
                continue;
            }
            let lo_pg = (l - m_va) / PAGE_SIZE;
            let hi_pg = (h - m_va).div_ceil(PAGE_SIZE);
            let mut m = maps.remove(i);
            m.runs(lo_pg, hi_pg, |rva, rsize| root.unmap(rva, rsize));
            if lo <= m_va && end >= m_va.saturating_add(m_size) {
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
                maps.push(right);
            }
            maps.push(m);
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
        let Some(end) = va.as_usize().checked_add(size) else {
            return false;
        };
        let mut at = va.as_usize();
        while at < end {
            let Some(map) = self.resolve_ref(VirtAddr::wrap(at)) else {
                return false;
            };
            if !allowed(map) {
                return false;
            }
            at = end.min(map.va.as_usize() + map.size.get());
        }
        true
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
        let end = va.as_usize().saturating_add(size);
        let span = |m: &Map| {
            let s = m.va.as_usize();
            let lo = va.as_usize().max(s);
            let hi = end.min(s.saturating_add(m.size.get()));
            (lo < hi).then_some((s, lo, hi))
        };
        let covered: usize = self
            .maps
            .iter()
            .filter_map(|m| span(m))
            .map(|(_, lo, hi)| hi - lo)
            .sum();
        if covered != size {
            return Err(MapError::NoRegion);
        }
        {
            let root = &self.root;
            for m in self.maps.iter() {
                if let Origin::Limited { ceiling } = &m.origin {
                    if span(m).is_some() && !ceiling.contains(flags & (PteFlags::R | PteFlags::W | PteFlags::X)) {
                        return Err(MapError::WidenDenied);
                    }
                }
                if let Origin::Backed { ceiling, .. } = &m.origin {
                    if span(m).is_some() && !ceiling.contains(flags & (PteFlags::R | PteFlags::W | PteFlags::X)) {
                        return Err(MapError::WidenDenied);
                    }
                }
                if !m.is_borrowed() {
                    continue;
                }
                let Some((s, lo, hi)) = span(m) else { continue };
                let lo_pg = (lo - s) / PAGE_SIZE;
                let hi_pg = (hi - s).div_ceil(PAGE_SIZE);
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
        for (index, boundary) in [va.as_usize(), end].into_iter().enumerate() {
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
        self.maps
            .try_reserve(splits.iter().flatten().count())
            .map_err(|_| MapError::OutOfMemory)?;
        for (boundary, mut right) in splits.into_iter().flatten() {
            let map = self
                .resolve_mut(VirtAddr::wrap(boundary))
                .expect("split mapping");
            let first = (boundary - map.va.as_usize()) / PAGE_SIZE;
            map.frames.move_tail(first, first, &mut right.frames);
            map.size = core::num::NonZeroUsize::new(first * PAGE_SIZE).expect("split prefix");
            self.maps.push(right);
        }
        let mut fault: Option<MapError> = None;
        let root = &mut self.root;
        for m in self.maps.iter_mut() {
            let Some((s, lo, hi)) = span(m) else { continue };
            let lo_pg = (lo - s) / PAGE_SIZE;
            let hi_pg = (hi - s).div_ceil(PAGE_SIZE);
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
        }
        match fault {
            Some(e) => Err(e),
            None => Ok(()),
        }
    }

    pub(crate) fn overlaps(&self, start: VirtAddr, size: usize) -> bool {
        let end = start.as_usize().saturating_add(size);
        self.maps.iter().any(|m| {
            start.as_usize() < m.va.as_usize().saturating_add(m.size.get()) && end > m.va.as_usize()
        })
    }

    pub(super) fn resolve_ref(&self, vaddr: VirtAddr) -> Option<&Map> {
        self.maps
            .iter()
            .rev()
            .find(|m| m.contains(vaddr))
            .map(Box::as_ref)
    }

    pub(super) fn resolve_mut(&mut self, vaddr: VirtAddr) -> Option<&mut Map> {
        self.maps
            .iter_mut()
            .rev()
            .find(|m| m.contains(vaddr))
            .map(Box::as_mut)
    }

    pub(super) fn translate(&self, vaddr: VirtAddr) -> Option<(PhysAddr, PteFlags)> {
        self.root
            .walk_ref(vaddr)
            .map(|x| (x.0 + vaddr.offset(), x.1))
            .ok()
    }

    #[cfg(debug_assertions)]
    pub(crate) fn audit(&self) {
        for m in &self.maps {
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

fn intersect(m: &Map, lo: usize, end: usize) -> Option<(usize, usize)> {
    let s = m.va.as_usize();
    let l = lo.max(s);
    let h = end.min(s.saturating_add(m.size.get()));
    (l < h).then(|| ((l - s) / PAGE_SIZE, (h - s).div_ceil(PAGE_SIZE)))
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
                self.inner.maps.retain(|map| map.va != base);
            }
            return;
        }
        for j in 0..self.installed {
            self.inner.root.unmap(self.va + j * PAGE_SIZE, PAGE_SIZE);
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
                self.inner.maps.retain(|m| m.va != va);
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
                self.maps.retain(|map| map.va != base);
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
