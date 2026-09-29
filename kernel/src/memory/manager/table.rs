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
    #[error("DRAM identity map overlaps the user stack window")]
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
        if level == 0 {
            for (i, e) in self.page.entries.iter().enumerate() {
                if e.is_valid() {
                    visit(VirtAddr::from_raw(node_va + (i << PAGE_SHIFT)));
                }
            }
            return;
        }
        let shift = PAGE_SHIFT + 9 * level;
        for (slot, child) in &self.children {
            child.mapped(level - 1, node_va + (slot << shift), visit);
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
        let mut node = self;
        for level in (0..levels).rev() {
            let idx = vaddr.vpn(level as u8);
            if level == 0 {
                return Ok(&mut node.page.entries[idx]);
            }
            let valid = node.page.entries[idx].is_valid();
            if !valid {
                if !alloc {
                    return Err(MapError::NotMapped);
                }
                let child = Self::leaf()?;
                let ppn = child.ppn() as u64;
                node.children
                    .try_reserve(1)
                    .map_err(|_| MapError::OutOfMemory)?;
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
        unreachable!("loop covers 0..levels")
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
                let pa = PhysAddr::from_raw(pte.paddr() as usize);
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
        let levels = super::mode::levels();
        let mut node = self;
        for level in (0..levels).rev() {
            let idx = vaddr.vpn(level as u8);
            let e = &node.page.entries[idx];
            if level == 0 {
                return if e.is_valid() && e.is_leaf() {
                    Ok((PhysAddr::from_raw(e.paddr() as usize), e.flags()))
                } else {
                    Err(MapError::NotMapped)
                };
            }
            if !e.is_valid() || e.is_leaf() {
                return Err(MapError::NotMapped);
            }
            node = &node
                .children
                .iter()
                .find(|(s, _)| *s == idx)
                .ok_or(MapError::NotMapped)?
                .1;
        }
        unreachable!("loop covers 0..levels")
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

        let pages = size / PAGE_SIZE;
        for i in 0..pages {
            let va = vaddr + i * PAGE_SIZE;
            let pa = paddr + i * PAGE_SIZE;
            let leaf = self.walk_mut(va, true, super::mode::levels())?;
            if leaf.is_valid() {
                return Err(MapError::AlreadyMapped);
            }
            let ppn = (pa.as_usize() >> PAGE_SHIFT) as u64;
            leaf.set(ppn, flags | PteFlags::V);
        }
        Ok(())
    }

    pub(crate) fn protect(
        &mut self,
        vaddr: VirtAddr,
        size: usize,
        flags: PteFlags,
    ) -> Result<(), MapError> {
        if size == 0 || vaddr.offset() != 0 || size & (PAGE_SIZE - 1) != 0 {
            return Err(MapError::NotAligned);
        }
        let pages = size / PAGE_SIZE;
        for i in 0..pages {
            let va = vaddr + i * PAGE_SIZE;
            let leaf = self.walk_mut(va, false, super::mode::levels())?;
            leaf.set_flags(flags);
        }
        Ok(())
    }

    pub(crate) fn unmap(&mut self, vaddr: VirtAddr, size: usize) {
        if size == 0 || vaddr.offset() != 0 || size & (PAGE_SIZE - 1) != 0 {
            return;
        }
        let pages = size / PAGE_SIZE;
        for i in 0..pages {
            if let Ok(leaf) = self.walk_mut(vaddr + i * PAGE_SIZE, false, super::mode::levels()) {
                leaf.clear();
            }
        }
        let geo = super::mode::geometry(super::mode::mode());
        let mask = (1usize << geo.va_bits) - 1;
        Self::recycle(
            self,
            (geo.levels - 1) as usize,
            0,
            vaddr.as_usize() & mask,
            (vaddr.as_usize() + size) & mask,
        );
    }

    fn recycle(&mut self, level: usize, node_va: usize, start: usize, end: usize) -> bool {
        if level > 0 {
            let span = 1usize << (12 + 9 * level);
            let node_end = node_va.saturating_add(span << 9);
            if end <= node_va || start >= node_end {
                return false;
            }
            let shift = 12 + 9 * level;
            let first = if start > node_va {
                (start - node_va) >> shift
            } else {
                0
            };
            let last = if end < node_end {
                (end - node_va - 1) >> shift
            } else {
                511
            };
            let mut remove: Vec<usize> = Vec::new();
            for (i, (slot, child)) in self.children.iter_mut().enumerate() {
                if *slot < first || *slot > last {
                    continue;
                }
                let child_va = node_va + (*slot << shift);
                if child.recycle(level - 1, child_va, start, end) {
                    self.page.entries[*slot].clear();
                    remove.push(i);
                }
            }
            for i in remove.into_iter().rev() {
                self.children.swap_remove(i);
            }
        }
        self.page.entries.iter().all(|e| !e.is_valid())
    }
}
