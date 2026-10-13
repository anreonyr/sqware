use alloc::vec::Vec;
use alloc::{alloc::Allocator, boxed::Box};
use fack::prelude::Error;

use crate::memory::{PAGE_SHIFT, PAGE_SIZE};

use super::{
    addr::{PhysAddr, VirtAddr},
    entry::{PageTableEntry, PteFlags},
};

#[derive(Error, Debug, Clone, Copy, PartialEq, Eq)]
pub enum MapError {
    #[error("physical frame allocator exhausted")]
    OutOfMemory,
    #[error("virtual address already mapped")]
    AlreadyMapped,
    #[error("address not page-aligned")]
    NotAligned,
    #[error("page table entry not mapped")]
    NotMapped,
    #[error("virtual address not in any declared map")]
    NoRegion,
    #[error("DRAM identity map exceeds the lower address half")]
    DramOverlap,
    #[error("cannot widen a borrowed mapping")]
    WidenDenied,
    #[error("address-space shootdown failed")]
    Shootdown,
    #[error("kernel address space has permanent lifetime")]
    Pinned,
}

#[repr(C, align(4096))]
#[derive(Debug)]
pub(crate) struct PageTable {
    pub(crate) entries: [PageTableEntry; 512],
}

impl Default for PageTable {
    fn default() -> Self {
        Self {
            entries: [PageTableEntry::default(); 512],
        }
    }
}

impl PageTable {
    pub(crate) fn new() -> Result<Box<PageTable, &'static dyn Allocator>, MapError> {
        let page = crate::tag!(Table, unsafe {
            Box::try_new_zeroed_in(crate::memory::allocator::frame::allocator())
                .map_err(|_| MapError::OutOfMemory)?
                .assume_init()
        });
        TABLE_LIVE.fetch_add(1, core::sync::atomic::Ordering::Relaxed);
        Ok(page)
    }
}

pub(crate) static TABLE_LIVE: core::sync::atomic::AtomicUsize =
    core::sync::atomic::AtomicUsize::new(0);

#[derive(Debug)]
pub(crate) struct TableNode {
    pub(crate) page: Box<PageTable, &'static dyn Allocator>,
    children: Vec<(usize, TableNode)>,
}

impl Drop for TableNode {
    fn drop(&mut self) {
        TABLE_LIVE.fetch_sub(1, core::sync::atomic::Ordering::Relaxed);
    }
}

impl TableNode {
    pub(crate) fn root() -> Result<Self, MapError> {
        Ok(Self {
            page: PageTable::new()?,
            children: Vec::new(),
        })
    }

    pub(crate) fn ppn(&self) -> usize {
        Box::as_ptr(&self.page) as usize >> PAGE_SHIFT
    }

    #[cfg(debug_assertions)]
    pub(crate) fn count(&self) -> usize {
        1 + self.children.iter().map(|(_, c)| c.count()).sum::<usize>()
    }

    pub(crate) fn node_count(&self) -> usize {
        1 + self
            .children
            .iter()
            .map(|(_, child)| child.node_count())
            .sum::<usize>()
    }

    #[cfg(debug_assertions)]
    pub(crate) fn visit_leaves(
        &self,
        level: usize,
        node_va: usize,
        geo: super::mode::Geo,
        visit: &mut impl FnMut(VirtAddr, PhysAddr, PteFlags, usize),
    ) {
        for (slot, child) in &self.children {
            let entry = self.page.entries[*slot];
            assert!(entry.is_branch(), "child table lacks a branch PTE");
            assert_eq!(
                entry.paddr() as usize,
                child.ppn() << PAGE_SHIFT,
                "branch PTE points at the wrong table"
            );
        }
        let span = Self::span(level);
        for (slot, entry) in self.page.entries.iter().enumerate() {
            if !entry.is_valid() {
                continue;
            }
            let base = node_va + slot * span;
            if entry.is_leaf() {
                assert_eq!(
                    entry.paddr() as usize & (span - 1),
                    0,
                    "misaligned superpage PTE"
                );
                let mask = (1usize << geo.va_bits) - 1;
                let canonical = if base & (1usize << (geo.va_bits - 1)) != 0 {
                    base | !mask
                } else {
                    base
                };
                visit(
                    VirtAddr::from_raw(canonical),
                    PhysAddr::from_raw(entry.paddr() as usize),
                    entry.flags(),
                    span,
                );
            } else if entry.is_branch() {
                assert!(level > 0, "branch PTE at the final page-table level");
                let child = self
                    .children
                    .iter()
                    .find(|(s, _)| *s == slot)
                    .map(|(_, child)| child)
                    .expect("branch PTE has no owned child table");
                child.visit_leaves(level - 1, base, geo, visit);
            }
        }
    }

    fn leaf() -> Result<Self, MapError> {
        Ok(Self {
            page: PageTable::new()?,
            children: Vec::new(),
        })
    }

    pub(crate) fn walk_mut(
        &mut self,
        vaddr: VirtAddr,
        alloc: bool,
        levels: usize,
    ) -> Result<&mut PageTableEntry, MapError> {
        self.entry_at(vaddr, 0, alloc, levels)
    }

    fn span(level: usize) -> usize {
        1usize << (PAGE_SHIFT + 9 * level)
    }

    fn entry_at(
        &mut self,
        va: VirtAddr,
        target: usize,
        alloc: bool,
        levels: usize,
    ) -> Result<&mut PageTableEntry, MapError> {
        let mut node = self;
        for level in (target..levels).rev() {
            let idx = va.vpn(level as u8);
            if level == target {
                return Ok(&mut node.page.entries[idx]);
            }
            let entry = node.page.entries[idx];
            if entry.is_leaf() && entry.is_valid() {
                return Err(MapError::AlreadyMapped);
            }
            if !entry.is_valid() {
                if !alloc {
                    return Err(MapError::NotMapped);
                }
                let child = Self::leaf()?;
                node.children
                    .try_reserve(1)
                    .map_err(|_| MapError::OutOfMemory)?;
                let ppn = child.ppn() as u64;
                node.children.push((idx, child));
                node.page.entries[idx].set(ppn, PteFlags::V);
            }
            node = &mut node
                .children
                .iter_mut()
                .find(|(s, _)| *s == idx)
                .expect("child exists (PTE ↔ tree invariant)")
                .1;
        }
        unreachable!("page table target level")
    }

    fn leaf_ref_geometry(
        &self,
        va: VirtAddr,
        geo: super::mode::Geo,
    ) -> Result<(PageTableEntry, usize), MapError> {
        let mut node = self;
        for level in (0..geo.levels as usize).rev() {
            let idx = va.vpn(level as u8);
            let entry = node.page.entries[idx];
            if !entry.is_valid() || entry.is_leaf() {
                return Ok((entry, level));
            }
            node = &node
                .children
                .iter()
                .find(|(slot, _)| *slot == idx)
                .ok_or(MapError::NotMapped)?
                .1;
        }
        Err(MapError::NotMapped)
    }

    fn split_at_geometry(
        &mut self,
        va: VirtAddr,
        target: usize,
        allocate: &mut impl FnMut() -> Result<Self, MapError>,
        geo: super::mode::Geo,
    ) -> Result<(), MapError> {
        let mut node = self;
        for level in (target + 1..geo.levels as usize).rev() {
            let idx = va.vpn(level as u8);
            let entry = node.page.entries[idx];
            if !entry.is_valid() {
                return Ok(());
            }
            if entry.is_leaf() {
                let mut child = allocate()?;
                let span = Self::span(level - 1);
                for (i, leaf) in child.page.entries.iter_mut().enumerate() {
                    let pa = entry.paddr() as usize + i * span;
                    leaf.set((pa >> PAGE_SHIFT) as u64, entry.flags());
                }
                node.children
                    .try_reserve(1)
                    .map_err(|_| MapError::OutOfMemory)?;
                let ppn = child.ppn() as u64;
                node.children.push((idx, child));
                core::sync::atomic::fence(core::sync::atomic::Ordering::Release);
                node.page.entries[idx].set(ppn, PteFlags::V);
            }
            node = &mut node
                .children
                .iter_mut()
                .find(|(s, _)| *s == idx)
                .expect("split child exists")
                .1;
        }
        Ok(())
    }

    fn prepare_with(
        &mut self,
        va: VirtAddr,
        size: usize,
        allocate: &mut impl FnMut() -> Result<Self, MapError>,
        geo: super::mode::Geo,
    ) -> Result<(), MapError> {
        if va.offset() != 0 || size & (PAGE_SIZE - 1) != 0 {
            return Err(MapError::NotAligned);
        }
        if size == 0 {
            return Ok(());
        }
        for (point, remaining, at_start) in
            [(va, size, true), (va + (size - PAGE_SIZE), size, false)]
        {
            loop {
                let (entry, level) = self.leaf_ref_geometry(point, geo)?;
                if !entry.is_valid() || level == 0 {
                    break;
                }
                let span = Self::span(level);
                let offset = point.as_usize() & (span - 1);
                let covered = if at_start {
                    offset == 0 && remaining >= span
                } else {
                    offset + PAGE_SIZE == span && remaining >= span
                };
                if covered {
                    break;
                }
                self.split_at_geometry(point, level - 1, allocate, geo)?;
            }
        }
        Ok(())
    }

    pub(crate) fn prepare_geometry(
        &mut self,
        va: VirtAddr,
        size: usize,
        geo: super::mode::Geo,
    ) -> Result<(), MapError> {
        self.prepare_with(va, size, &mut Self::leaf, geo)
    }

    pub(crate) fn walk_raw(
        root: PhysAddr,
        page_va: VirtAddr,
        ok: impl Fn(PhysAddr) -> bool,
    ) -> Option<(PhysAddr, PteFlags)> {
        let levels = super::mode::levels();
        let mut tbl = root;
        if !ok(tbl) {
            return None;
        }
        for level in (0..levels).rev() {
            let pte = unsafe {
                *((tbl.as_usize() + page_va.vpn(level as u8) * 8) as *const PageTableEntry)
            };
            if !pte.is_valid() {
                return None;
            }
            if pte.is_leaf() {
                let span = Self::span(level);
                let base = pte.paddr() as usize;
                if base & (span - 1) != 0 {
                    return None;
                }
                let pa =
                    PhysAddr::from_raw(base + (page_va.as_usize() & (span - 1) & !(PAGE_SIZE - 1)));
                return ok(pa).then_some((pa, pte.flags()));
            }
            tbl = PhysAddr::from_raw(pte.paddr() as usize);
            if !ok(tbl) {
                return None;
            }
        }
        None
    }

    #[cfg(debug_assertions)]
    pub(crate) fn leaf_span_geometry(
        &self,
        vaddr: VirtAddr,
        geo: super::mode::Geo,
    ) -> Option<usize> {
        let (entry, level) = self.leaf_ref_geometry(vaddr, geo).ok()?;
        (entry.is_valid() && entry.is_leaf()).then(|| Self::span(level))
    }

    pub(crate) fn walk_ref_geometry(
        &self,
        vaddr: VirtAddr,
        geo: super::mode::Geo,
    ) -> Result<(PhysAddr, PteFlags), MapError> {
        let (entry, level) = self.leaf_ref_geometry(vaddr, geo)?;
        if !entry.is_valid() || !entry.is_leaf() {
            return Err(MapError::NotMapped);
        }
        let span = Self::span(level);
        let base = entry.paddr() as usize;
        if base & (span - 1) != 0 {
            return Err(MapError::NotMapped);
        }
        let offset = vaddr.as_usize() & (span - 1) & !(PAGE_SIZE - 1);
        Ok((PhysAddr::from_raw(base + offset), entry.flags()))
    }

    pub(crate) fn prepare_range_geometry(
        &mut self,
        va: VirtAddr,
        pa: PhysAddr,
        size: usize,
        geo: super::mode::Geo,
    ) -> Result<(), MapError> {
        if va.offset() != 0 || !pa.is_aligned() || size == 0 || size & (PAGE_SIZE - 1) != 0 {
            return Err(MapError::NotAligned);
        }
        let levels = geo.levels as usize;
        let mut offset = 0;
        while offset < size {
            let v = va + offset;
            let p = pa + offset;
            let mut target = 2.min(levels - 1);
            while target > 0
                && (size - offset < Self::span(target)
                    || (v.as_usize() | p.as_usize()) & (Self::span(target) - 1) != 0)
            {
                target -= 1;
            }
            loop {
                let entry = self.entry_at(v, target, true, levels)?;
                if !entry.is_valid() {
                    offset += Self::span(target);
                    break;
                }
                if target == 0 || entry.is_leaf() {
                    return Err(MapError::AlreadyMapped);
                }
                target -= 1;
            }
        }
        Ok(())
    }

    pub(crate) fn map_range_prepared_geometry(
        &mut self,
        va: VirtAddr,
        pa: PhysAddr,
        size: usize,
        flags: PteFlags,
        geo: super::mode::Geo,
    ) {
        debug_assert_eq!(va.offset(), 0);
        debug_assert!(pa.is_aligned());
        let levels = geo.levels as usize;
        let mut offset = 0;
        while offset < size {
            let v = va + offset;
            let p = pa + offset;
            let mut target = 2.min(levels - 1);
            while target > 0
                && (size - offset < Self::span(target)
                    || (v.as_usize() | p.as_usize()) & (Self::span(target) - 1) != 0)
            {
                target -= 1;
            }
            let entry = self
                .entry_at(v, target, false, levels)
                .expect("prepared range path");
            debug_assert!(!entry.is_valid());
            entry.set((p.as_usize() >> PAGE_SHIFT) as u64, flags | PteFlags::V);
            offset += Self::span(target);
        }
    }

    pub(crate) fn set_permissions_prepared_geometry(
        &mut self,
        va: VirtAddr,
        flags: PteFlags,
        geo: super::mode::Geo,
    ) {
        let leaf = self
            .walk_mut(va, false, geo.levels as usize)
            .expect("preflighted page-table path");
        debug_assert!(
            leaf.is_valid() && leaf.is_leaf(),
            "preflighted leaf mapping"
        );
        leaf.set_flags((leaf.flags() - (PteFlags::R | PteFlags::W | PteFlags::X)) | flags);
    }

    #[cfg(debug_assertions)]
    pub(crate) fn protect_permissions_geometry(
        &mut self,
        va: VirtAddr,
        size: usize,
        flags: PteFlags,
        geo: super::mode::Geo,
    ) -> Result<(), MapError> {
        self.prepare_geometry(va, size, geo)?;
        let mut offset = 0;
        while offset < size {
            let (entry, level) = self.leaf_ref_geometry(va + offset, geo)?;
            if !entry.is_valid() || !entry.is_leaf() {
                return Err(MapError::NotMapped);
            }
            offset += Self::span(level);
        }
        offset = 0;
        while offset < size {
            let point = va + offset;
            let (_, level) = self.leaf_ref_geometry(point, geo)?;
            let leaf = self.entry_at(point, level, false, geo.levels as usize)?;
            leaf.set_flags((leaf.flags() - (PteFlags::R | PteFlags::W | PteFlags::X)) | flags);
            offset += Self::span(level);
        }
        Ok(())
    }

    pub(crate) fn unmap_keep_tables_geometry(
        &mut self,
        va: VirtAddr,
        size: usize,
        geo: super::mode::Geo,
    ) -> Result<(), MapError> {
        self.prepare_geometry(va, size, geo)?;
        let mask = (1usize << geo.va_bits) - 1;
        let start = va.as_usize() & mask;
        self.clear_leaf_range((geo.levels - 1) as usize, 0, start, start + size);
        Ok(())
    }

    pub(crate) fn reserve_retired_tables(
        &self,
        retired: &mut Vec<TableNode>,
    ) -> Result<(), MapError> {
        retired
            .try_reserve(self.node_count())
            .map_err(|_| MapError::OutOfMemory)
    }

    pub(crate) fn reclaim_empty_tables(&mut self, retired: &mut Vec<TableNode>) {
        debug_assert!(retired.capacity() - retired.len() >= self.node_count());
        self.prune_empty(retired);
    }

    fn clear_leaf_range(&mut self, level: usize, node_va: usize, start: usize, end: usize) {
        let span = Self::span(level);
        let node_end = node_va + span * 512;
        if end <= node_va || start >= node_end {
            return;
        }
        let first = (start.max(node_va) - node_va) / span;
        let last = (end.min(node_end) - node_va - 1) / span;
        for slot in first..=last {
            let entry = self.page.entries[slot];
            if !entry.is_valid() {
                continue;
            }
            if entry.is_leaf() {
                let base = node_va + slot * span;
                debug_assert!(
                    start <= base && end >= base + span,
                    "unmap boundary not prepared"
                );
                self.page.entries[slot].clear();
            } else if let Some((_, child)) = self.children.iter_mut().find(|(s, _)| *s == slot) {
                child.clear_leaf_range(level - 1, node_va + slot * span, start, end);
            }
        }
    }

    fn prune_empty(&mut self, retired: &mut Vec<TableNode>) -> bool {
        let mut index = 0;
        while index < self.children.len() {
            let (_, child) = &mut self.children[index];
            if child.prune_empty(retired) {
                let (slot, child) = self.children.swap_remove(index);
                self.page.entries[slot].clear();
                retired.push(child);
            } else {
                index += 1;
            }
        }
        self.page.entries.iter().all(|entry| !entry.is_valid())
    }
}

#[cfg(debug_assertions)]
pub(crate) fn large_pages_accept() {
    const MEGA: usize = 1 << 21;
    const GIGA: usize = 1 << 30;
    let va = VirtAddr::wrap(GIGA);
    let pa = PhysAddr::from_raw(2 * GIGA);
    let rw = PteFlags::V | PteFlags::R | PteFlags::W | PteFlags::A | PteFlags::D;
    let ro = rw - PteFlags::W;
    let geo = super::mode::geometry(super::mode::mode());
    let levels = geo.levels as usize;
    let mut root = TableNode::root().expect("large page root");
    root.prepare_range_geometry(va, pa, GIGA, geo)
        .expect("gigapage map");
    root.map_range_prepared_geometry(va, pa, GIGA, rw, geo);
    assert_eq!(root.count(), levels - 2);
    assert_eq!(root.leaf_ref_geometry(va, geo).unwrap().1, 2);
    for offset in [0, PAGE_SIZE, MEGA + PAGE_SIZE, GIGA - PAGE_SIZE] {
        let point = va + offset;
        assert_eq!(root.walk_ref_geometry(point, geo).unwrap().0, pa + offset);
        let physical_root = PhysAddr::from_raw(root.ppn() << PAGE_SHIFT);
        assert_eq!(
            TableNode::walk_raw(physical_root, point, |_| true)
                .unwrap()
                .0,
            pa + offset
        );
    }
    assert_eq!(
        root.prepare_range_geometry(va + PAGE_SIZE, pa, PAGE_SIZE, geo),
        Err(MapError::AlreadyMapped)
    );
    let count = root.count();
    {
        let guard = crate::memory::allocator::NoAllocation::enter();
        root.protect_permissions_geometry(va, GIGA, ro, geo)
            .expect("whole gigapage protect");
        root.protect_permissions_geometry(va, GIGA, PteFlags::R | PteFlags::W, geo)
            .unwrap();
        assert_eq!(root.count(), count);
        drop(guard);
    }
    let point = va + MEGA + PAGE_SIZE;
    let mut allocations = 0;
    let result = root.prepare_with(
        point,
        PAGE_SIZE,
        &mut || {
            allocations += 1;
            if allocations == 2 {
                Err(MapError::OutOfMemory)
            } else {
                TableNode::leaf()
            }
        },
        geo,
    );
    assert_eq!(result, Err(MapError::OutOfMemory));
    assert_eq!(allocations, 2);
    assert_eq!(root.count(), count + 1);
    for offset in [0, MEGA, MEGA + PAGE_SIZE, GIGA - PAGE_SIZE] {
        let (physical, flags) = root.walk_ref_geometry(va + offset, geo).unwrap();
        assert_eq!(physical, pa + offset);
        assert_eq!(flags.bits(), rw.bits());
    }
    root.protect_permissions_geometry(point, PAGE_SIZE, ro, geo)
        .expect("retry partial protect");
    assert_eq!(root.count(), count + 2);
    assert_eq!(root.leaf_ref_geometry(point, geo).unwrap().1, 0);
    assert_eq!(root.leaf_ref_geometry(va, geo).unwrap().1, 1);
    assert!(
        !root
            .walk_ref_geometry(point, geo)
            .unwrap()
            .1
            .contains(PteFlags::W)
    );
    for neighbor in [point - PAGE_SIZE, point + PAGE_SIZE] {
        assert!(
            root.walk_ref_geometry(neighbor, geo)
                .unwrap()
                .1
                .contains(PteFlags::W)
        );
    }
    let mut retired = Vec::new();
    root.reserve_retired_tables(&mut retired).unwrap();
    root.prepare_geometry(point, PAGE_SIZE, geo)
        .expect("partial unmap");
    root.unmap_keep_tables_geometry(point, PAGE_SIZE, geo)
        .expect("partial unmap");
    root.reclaim_empty_tables(&mut retired);
    assert!(root.walk_ref_geometry(point, geo).is_err());
    root.prepare_range_geometry(point, pa + MEGA + PAGE_SIZE, PAGE_SIZE, geo)
        .expect("refill small hole");
    root.map_range_prepared_geometry(point, pa + MEGA + PAGE_SIZE, PAGE_SIZE, rw, geo);
    let mut retired = Vec::new();
    root.reserve_retired_tables(&mut retired).unwrap();
    let guard = crate::memory::allocator::NoAllocation::enter();
    root.prepare_geometry(va, GIGA, geo)
        .expect("whole gigapage removal");
    root.unmap_keep_tables_geometry(va, GIGA, geo)
        .expect("whole gigapage removal");
    root.reclaim_empty_tables(&mut retired);
    assert_eq!(root.count(), 1);
    drop(guard);

    let start = va - PAGE_SIZE;
    root.prepare_range_geometry(start, pa - PAGE_SIZE, MEGA + 2 * PAGE_SIZE, geo)
        .expect("mixed map");
    root.map_range_prepared_geometry(start, pa - PAGE_SIZE, MEGA + 2 * PAGE_SIZE, rw, geo);
    for (point, expected) in [(start, 0), (va, 1), (va + MEGA, 0)] {
        assert_eq!(root.leaf_ref_geometry(point, geo).unwrap().1, expected);
    }
    root.protect_permissions_geometry(va + MEGA - PAGE_SIZE, 2 * PAGE_SIZE, ro, geo)
        .expect("cross leaf boundary");
    assert!(
        !root
            .walk_ref_geometry(va + MEGA - PAGE_SIZE, geo)
            .unwrap()
            .1
            .contains(PteFlags::W)
    );
    assert!(
        !root
            .walk_ref_geometry(va + MEGA, geo)
            .unwrap()
            .1
            .contains(PteFlags::W)
    );
    assert!(
        root.walk_ref_geometry(va + MEGA - 2 * PAGE_SIZE, geo)
            .unwrap()
            .1
            .contains(PteFlags::W)
    );
    let mut retired = Vec::new();
    root.reserve_retired_tables(&mut retired).unwrap();
    root.prepare_geometry(start, MEGA + 2 * PAGE_SIZE, geo)
        .unwrap();
    root.unmap_keep_tables_geometry(start, MEGA + 2 * PAGE_SIZE, geo)
        .unwrap();
    root.reclaim_empty_tables(&mut retired);
    assert_eq!(root.count(), 1);

    root.prepare_range_geometry(va + PAGE_SIZE, pa + PAGE_SIZE, PAGE_SIZE, geo)
        .unwrap();
    root.map_range_prepared_geometry(va + PAGE_SIZE, pa + PAGE_SIZE, PAGE_SIZE, rw, geo);
    root.prepare_range_geometry(va, pa, PAGE_SIZE, geo).unwrap();
    root.map_range_prepared_geometry(va, pa, PAGE_SIZE, rw, geo);
    root.prepare_range_geometry(
        va + 2 * PAGE_SIZE,
        pa + 2 * PAGE_SIZE,
        MEGA - 2 * PAGE_SIZE,
        geo,
    )
    .unwrap();
    root.map_range_prepared_geometry(
        va + 2 * PAGE_SIZE,
        pa + 2 * PAGE_SIZE,
        MEGA - 2 * PAGE_SIZE,
        rw,
        geo,
    );
    assert_eq!(root.leaf_ref_geometry(va, geo).unwrap().1, 0);
    let mut retired = Vec::new();
    root.reserve_retired_tables(&mut retired).unwrap();
    root.prepare_geometry(va, MEGA, geo).unwrap();
    root.unmap_keep_tables_geometry(va, MEGA, geo).unwrap();
    root.reclaim_empty_tables(&mut retired);
    assert_eq!(root.count(), 1);

    let top = VirtAddr::wrap(usize::MAX - MEGA + 1);
    root.prepare_range_geometry(top, pa, MEGA, geo)
        .expect("top megapage");
    root.map_range_prepared_geometry(top, pa, MEGA, rw, geo);
    root.protect_permissions_geometry(top + (MEGA - PAGE_SIZE), PAGE_SIZE, ro, geo)
        .unwrap();
    assert_eq!(
        root.walk_ref_geometry(VirtAddr::wrap(usize::MAX), geo)
            .unwrap()
            .0,
        pa + MEGA - PAGE_SIZE
    );
    let mut retired = Vec::new();
    root.reserve_retired_tables(&mut retired).unwrap();
    root.prepare_geometry(top, MEGA, geo)
        .expect("top megapage removal");
    root.unmap_keep_tables_geometry(top, MEGA, geo)
        .expect("top megapage removal");
    root.reclaim_empty_tables(&mut retired);
    assert_eq!(root.count(), 1);
}
