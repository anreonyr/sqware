use alloc::vec::Vec;
use alloc::{alloc::Allocator, boxed::Box};
use fack::prelude::Error;

use crate::memory::{PAGE_SHIFT, PAGE_SIZE};

use super::{
    addr::{PhysAddr, VirtAddr},
    entry::{PageTableEntry, PteFlags},
};

pub(crate) type Frame = Box<[u8; PAGE_SIZE], &'static dyn Allocator>;

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
    #[error("span does not match segment state")]
    SegmentMismatch,
    #[error("cannot widen a borrowed mapping")]
    WidenDenied,
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

    #[cfg(debug_assertions)]
    pub(crate) fn mapped(&self, level: usize, node_va: usize, visit: &mut impl FnMut(VirtAddr)) {
        let span = Self::span(level);
        for (slot, entry) in self.page.entries.iter().enumerate() {
            if !entry.is_valid() { continue; }
            let base = node_va + slot * span;
            if entry.is_leaf() {
                for offset in (0..span).step_by(PAGE_SIZE) {
                    visit(VirtAddr::from_raw(base + offset));
                }
            } else if let Some((_, child)) = self.children.iter().find(|(s, _)| *s == slot) {
                child.mapped(level - 1, base, visit);
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

    fn span(level: usize) -> usize { 1usize << (PAGE_SHIFT + 9 * level) }

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
            if level == target { return Ok(&mut node.page.entries[idx]); }
            let entry = node.page.entries[idx];
            if entry.is_leaf() && entry.is_valid() { return Err(MapError::AlreadyMapped); }
            if !entry.is_valid() {
                if !alloc { return Err(MapError::NotMapped); }
                let child = Self::leaf()?;
                node.children.try_reserve(1).map_err(|_| MapError::OutOfMemory)?;
                let ppn = child.ppn() as u64;
                node.children.push((idx, child));
                node.page.entries[idx].set(ppn, PteFlags::V);
            }
            node = &mut node.children.iter_mut().find(|(s, _)| *s == idx)
                .expect("child exists (PTE ↔ tree invariant)").1;
        }
        unreachable!("page table target level")
    }

    fn leaf_ref(&self, va: VirtAddr) -> Result<(PageTableEntry, usize), MapError> {
        let mut node = self;
        for level in (0..super::mode::levels()).rev() {
            let idx = va.vpn(level as u8);
            let entry = node.page.entries[idx];
            if !entry.is_valid() || entry.is_leaf() { return Ok((entry, level)); }
            node = &node.children.iter().find(|(s, _)| *s == idx)
                .ok_or(MapError::NotMapped)?.1;
        }
        Err(MapError::NotMapped)
    }

    fn split_at(
        &mut self,
        va: VirtAddr,
        target: usize,
        allocate: &mut impl FnMut() -> Result<Self, MapError>,
    ) -> Result<(), MapError> {
        let mut node = self;
        for level in (target + 1..super::mode::levels()).rev() {
            let idx = va.vpn(level as u8);
            let entry = node.page.entries[idx];
            if !entry.is_valid() { return Ok(()); }
            if entry.is_leaf() {
                let mut child = allocate()?;
                let span = Self::span(level - 1);
                for (i, leaf) in child.page.entries.iter_mut().enumerate() {
                    let pa = entry.paddr() as usize + i * span;
                    leaf.set((pa >> PAGE_SHIFT) as u64, entry.flags());
                }
                node.children.try_reserve(1).map_err(|_| MapError::OutOfMemory)?;
                let ppn = child.ppn() as u64;
                node.children.push((idx, child));
                // Publish the complete child table before replacing the leaf.
                core::sync::atomic::fence(core::sync::atomic::Ordering::Release);
                node.page.entries[idx].set(ppn, PteFlags::V);
            }
            node = &mut node.children.iter_mut().find(|(s, _)| *s == idx)
                .expect("split child exists").1;
        }
        Ok(())
    }

    // Splitting preserves translations and flags, even if a later allocation fails.
    pub(crate) fn prepare(&mut self, va: VirtAddr, size: usize) -> Result<(), MapError> {
        self.prepare_with(va, size, &mut Self::leaf)
    }

    fn prepare_with(
        &mut self,
        va: VirtAddr,
        size: usize,
        allocate: &mut impl FnMut() -> Result<Self, MapError>,
    ) -> Result<(), MapError> {
        if va.offset() != 0 || size & (PAGE_SIZE - 1) != 0 { return Err(MapError::NotAligned); }
        if size == 0 { return Ok(()); }
        for (point, remaining, at_start) in [(va, size, true), (va + (size - PAGE_SIZE), size, false)] {
            loop {
                let (entry, level) = self.leaf_ref(point)?;
                if !entry.is_valid() || level == 0 { break; }
                let span = Self::span(level);
                let offset = point.as_usize() & (span - 1);
                let covered = if at_start { offset == 0 && remaining >= span }
                    else { offset + PAGE_SIZE == span && remaining >= span };
                if covered { break; }
                self.split_at(point, level - 1, allocate)?;
            }
        }
        Ok(())
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
            // SAFETY: tbl 已过 ok 校验；调用方保证只读、无并发写
            let pte = unsafe {
                *((tbl.as_usize() + page_va.vpn(level as u8) * 8) as *const PageTableEntry)
            };
            if !pte.is_valid() {
                return None;
            }
            if pte.is_leaf() {
                let span = Self::span(level);
                let base = pte.paddr() as usize;
                if base & (span - 1) != 0 { return None; }
                let pa = PhysAddr::from_raw(base + (page_va.as_usize() & (span - 1) & !(PAGE_SIZE - 1)));
                return ok(pa).then_some((pa, pte.flags()));
            }
            tbl = PhysAddr::from_raw(pte.paddr() as usize);
            if !ok(tbl) {
                return None;
            }
        }
        None
    }

    pub(crate) fn walk_ref(&self, vaddr: VirtAddr) -> Result<(PhysAddr, PteFlags), MapError> {
        let (entry, level) = self.leaf_ref(vaddr)?;
        if !entry.is_valid() || !entry.is_leaf() { return Err(MapError::NotMapped); }
        let span = Self::span(level);
        let base = entry.paddr() as usize;
        if base & (span - 1) != 0 { return Err(MapError::NotMapped); }
        let offset = vaddr.as_usize() & (span - 1) & !(PAGE_SIZE - 1);
        Ok((PhysAddr::from_raw(base + offset), entry.flags()))
    }

    pub(crate) fn map(
        &mut self,
        vaddr: VirtAddr,
        paddr: PhysAddr,
        size: usize,
        flags: PteFlags,
    ) -> Result<(), MapError> {
        if vaddr.offset() != 0 || !paddr.is_aligned() || size & (PAGE_SIZE - 1) != 0 {
            return Err(MapError::NotAligned);
        }

        let levels = super::mode::levels();
        let mut offset = 0usize;
        while offset < size {
            let va = vaddr + offset;
            let pa = paddr + offset;
            let mut target = 2.min(levels - 1);
            while target > 0 && (size - offset < Self::span(target)
                || (va.as_usize() | pa.as_usize()) & (Self::span(target) - 1) != 0)
            { target -= 1; }
            loop {
                let entry = self.entry_at(va, target, true, levels)?;
                if !entry.is_valid() {
                    entry.set((pa.as_usize() >> PAGE_SHIFT) as u64, flags | PteFlags::V);
                    offset += Self::span(target);
                    break;
                }
                if target == 0 || entry.is_leaf() { return Err(MapError::AlreadyMapped); }
                target -= 1;
            }
        }
        Ok(())
    }

    pub(crate) fn protect(&mut self, va: VirtAddr, size: usize, flags: PteFlags) -> Result<(), MapError> {
        self.protect_with(va, size, |_| flags)
    }

    pub(crate) fn protect_permissions(
        &mut self,
        va: VirtAddr,
        size: usize,
        flags: PteFlags,
    ) -> Result<(), MapError> {
        self.protect_with(va, size, |current|
            (current - (PteFlags::R | PteFlags::W | PteFlags::X)) | flags)
    }

    fn protect_with(
        &mut self,
        va: VirtAddr,
        size: usize,
        flags: impl Fn(PteFlags) -> PteFlags,
    ) -> Result<(), MapError> {
        self.prepare(va, size)?;
        let mut offset = 0usize;
        // Check coverage before changing any permissions.
        while offset < size {
            let (entry, level) = self.leaf_ref(va + offset)?;
            if !entry.is_valid() || !entry.is_leaf() { return Err(MapError::NotMapped); }
            offset += Self::span(level);
        }
        offset = 0;
        while offset < size {
            let point = va + offset;
            let (_, level) = self.leaf_ref(point)?;
            let leaf = self.entry_at(point, level, false, super::mode::levels())?;
            leaf.set_flags(flags(leaf.flags()));
            offset += Self::span(level);
        }
        Ok(())
    }

    pub(crate) fn unmap(&mut self, va: VirtAddr, size: usize) -> Result<(), MapError> {
        self.prepare(va, size)?;
        self.unmap_prepared(va, size);
        Ok(())
    }

    // The range boundaries must already be split, or cover whole leaves.
    pub(crate) fn unmap_prepared(&mut self, va: VirtAddr, size: usize) {
        debug_assert_eq!(va.offset(), 0);
        debug_assert_eq!(size & (PAGE_SIZE - 1), 0);
        if size == 0 { return; }
        let geo = super::mode::geometry(super::mode::mode());
        let mask = (1usize << geo.va_bits) - 1;
        let start = va.as_usize() & mask;
        self.clear((geo.levels - 1) as usize, 0, start, start + size);
    }

    fn clear(&mut self, level: usize, node_va: usize, start: usize, end: usize) -> bool {
        let span = Self::span(level);
        let node_end = node_va + span * 512;
        if end <= node_va || start >= node_end { return false; }
        let first = (start.max(node_va) - node_va) / span;
        let last = (end.min(node_end) - node_va - 1) / span;
        for slot in first..=last {
            let entry = &mut self.page.entries[slot];
            if entry.is_valid() && entry.is_leaf() {
                let base = node_va + slot * span;
                debug_assert!(start <= base && end >= base + span, "unmap boundary not prepared");
                entry.clear();
            }
        }
        let mut i = 0;
        while i < self.children.len() {
            let (slot, child) = &mut self.children[i];
            if *slot >= first && *slot <= last
                && child.clear(level - 1, node_va + *slot * span, start, end)
            {
                self.page.entries[*slot].clear();
                self.children.swap_remove(i);
            } else { i += 1; }
        }
        self.page.entries.iter().all(|e| !e.is_valid())
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
    let levels = super::mode::levels();
    let mut root = TableNode::root().expect("large page root");
    root.map(va, pa, GIGA, rw).expect("gigapage map");
    assert_eq!(root.count(), levels - 2);
    assert_eq!(root.leaf_ref(va).unwrap().1, 2);
    for offset in [0, PAGE_SIZE, MEGA + PAGE_SIZE, GIGA - PAGE_SIZE] {
        let point = va + offset;
        assert_eq!(root.walk_ref(point).unwrap().0, pa + offset);
        let physical_root = PhysAddr::from_raw(root.ppn() << PAGE_SHIFT);
        assert_eq!(TableNode::walk_raw(physical_root, point, |_| true).unwrap().0, pa + offset);
    }
    assert_eq!(root.map(va + PAGE_SIZE, pa, PAGE_SIZE, rw), Err(MapError::AlreadyMapped));
    let count = root.count();
    {
        let guard = crate::memory::allocator::NoAllocation::enter();
        root.protect(va, GIGA, ro).expect("whole gigapage protect");
        root.protect_permissions(va, GIGA, PteFlags::R | PteFlags::W).unwrap();
        assert_eq!(root.count(), count);
        drop(guard);
    }
    let point = va + MEGA + PAGE_SIZE;
    let mut allocations = 0;
    let result = root.prepare_with(point, PAGE_SIZE, &mut || {
        allocations += 1;
        if allocations == 2 { Err(MapError::OutOfMemory) } else { TableNode::leaf() }
    });
    assert_eq!(result, Err(MapError::OutOfMemory));
    assert_eq!(allocations, 2);
    assert_eq!(root.count(), count + 1);
    for offset in [0, MEGA, MEGA + PAGE_SIZE, GIGA - PAGE_SIZE] {
        let (physical, flags) = root.walk_ref(va + offset).unwrap();
        assert_eq!(physical, pa + offset);
        assert_eq!(flags.bits(), rw.bits());
    }
    root.protect(point, PAGE_SIZE, ro).expect("retry partial protect");
    assert_eq!(root.count(), count + 2);
    assert_eq!(root.leaf_ref(point).unwrap().1, 0);
    assert_eq!(root.leaf_ref(va).unwrap().1, 1);
    assert!(!root.walk_ref(point).unwrap().1.contains(PteFlags::W));
    for neighbor in [point - PAGE_SIZE, point + PAGE_SIZE] {
        assert!(root.walk_ref(neighbor).unwrap().1.contains(PteFlags::W));
    }
    root.unmap(point, PAGE_SIZE).expect("partial unmap");
    assert!(root.walk_ref(point).is_err());
    root.map(point, pa + MEGA + PAGE_SIZE, PAGE_SIZE, rw).expect("refill small hole");
    let guard = crate::memory::allocator::NoAllocation::enter();
    root.unmap(va, GIGA).expect("whole gigapage removal");
    assert_eq!(root.count(), 1);
    drop(guard);

    // Unaligned ends retain base pages around an aligned megapage.
    let start = va - PAGE_SIZE;
    root.map(start, pa - PAGE_SIZE, MEGA + 2 * PAGE_SIZE, rw).expect("mixed map");
    for (point, expected) in [(start, 0), (va, 1), (va + MEGA, 0)] {
        assert_eq!(root.leaf_ref(point).unwrap().1, expected);
    }
    root.protect(va + MEGA - PAGE_SIZE, 2 * PAGE_SIZE, ro).expect("cross leaf boundary");
    assert!(!root.walk_ref(va + MEGA - PAGE_SIZE).unwrap().1.contains(PteFlags::W));
    assert!(!root.walk_ref(va + MEGA).unwrap().1.contains(PteFlags::W));
    assert!(root.walk_ref(va + MEGA - 2 * PAGE_SIZE).unwrap().1.contains(PteFlags::W));
    root.unmap(start, MEGA + 2 * PAGE_SIZE).unwrap();
    assert_eq!(root.count(), 1);

    // An existing lower table must not be overwritten by a large leaf.
    root.map(va + PAGE_SIZE, pa + PAGE_SIZE, PAGE_SIZE, rw).unwrap();
    root.map(va, pa, PAGE_SIZE, rw).unwrap();
    root.map(va + 2 * PAGE_SIZE, pa + 2 * PAGE_SIZE, MEGA - 2 * PAGE_SIZE, rw).unwrap();
    assert_eq!(root.leaf_ref(va).unwrap().1, 0);
    root.unmap(va, MEGA).unwrap();
    assert_eq!(root.count(), 1);

    let top = VirtAddr::wrap(usize::MAX - MEGA + 1);
    root.map(top, pa, MEGA, rw).expect("top megapage");
    root.protect(top + (MEGA - PAGE_SIZE), PAGE_SIZE, ro).unwrap();
    assert_eq!(root.walk_ref(VirtAddr::wrap(usize::MAX)).unwrap().0, pa + MEGA - PAGE_SIZE);
    root.unmap(top, MEGA).expect("top megapage removal");
    assert_eq!(root.count(), 1);
}
