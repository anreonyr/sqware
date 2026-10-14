use alloc::boxed::Box;
use alloc::sync::Arc;
use alloc::vec::Vec;
use core::mem::ManuallyDrop;
use core::num::NonZeroUsize;
use core::sync::atomic::{AtomicBool, Ordering};

use riscv::register::satp;

use crate::core::memory::PAGE_SIZE;
use crate::machine::sync::{Level, SpinLock, SpinLockGuard};

use super::addr::{PhysAddr, VirtAddr};
use super::asid::{self, Asid};
use super::entry::PteFlags;
use super::mode;
use super::table::TableNode;
use super::{Frame, MapError};

mod borrowed_region;
mod index;
mod invalidation;
mod map;
mod phase;
mod state;

use index::IndexNode;
use invalidation::Invalidation;
use map::range::valid_leaf_flags;
use state::TreeState;

pub use borrowed_region::BorrowedRegion;
pub(crate) use map::{Map, MapRange};

pub(crate) struct Tree {
    asid: Asid,
    geometry: mode::Geo,
    state: ManuallyDrop<SpinLock<TreeState>>,
    asid_released: AtomicBool,
    published: AtomicBool,
}

impl Tree {
    fn ready_state(&self) -> Result<SpinLockGuard<'_, TreeState>, MapError> {
        loop {
            let state = self.state.lock();
            if state.phase.is_closed() {
                return Err(MapError::NoRegion);
            }
            match state.invalidation {
                Invalidation::Clean => return Ok(state),
                Invalidation::Failed => return Err(MapError::Shootdown),
                Invalidation::Dirty | Invalidation::Fencing => {}
            }
            drop(state);
            self.finish_invalidation()?;
        }
    }

    pub fn new() -> Result<Arc<Self>, MapError> {
        let root = TableNode::root()?;
        let asid = Asid::try_allocate().ok_or(MapError::OutOfMemory)?;
        let tree = Arc::try_new(Self {
            asid,
            geometry: mode::geometry(mode::mode()),
            state: ManuallyDrop::new(SpinLock::new_level(Level::Tree, TreeState::new(root))),
            asid_released: AtomicBool::new(false),
            published: AtomicBool::new(false),
        });
        match tree {
            Ok(mut tree) => {
                Arc::get_mut(&mut tree)
                    .expect("new Arc is unique")
                    .published
                    .store(true, Ordering::Release);
                Ok(tree)
            }
            Err(_) => Err(MapError::OutOfMemory),
        }
    }

    pub fn new_kernel() -> Result<Arc<Self>, MapError> {
        let root = TableNode::root()?;
        let tree = Arc::try_new(Self {
            asid: Asid::kernel(),
            geometry: mode::geometry(mode::mode()),
            state: ManuallyDrop::new(SpinLock::new_level(Level::Tree, TreeState::new(root))),
            asid_released: AtomicBool::new(true),
            published: AtomicBool::new(false),
        });
        let mut tree = tree.map_err(|_| MapError::OutOfMemory)?;
        Arc::get_mut(&mut tree)
            .expect("new Arc is unique")
            .published
            .store(true, Ordering::Release);
        Ok(tree)
    }

    pub fn asid(&self) -> Asid {
        self.asid
    }
    #[cfg(debug_assertions)]
    pub fn geometry(&self) -> mode::Geo {
        self.geometry
    }
    pub fn root_ppn(&self) -> usize {
        self.state.lock().root.ppn()
    }
    pub fn satp(&self) -> satp::Satp {
        satp::Satp::from_bits(
            (mode::mode().into_usize() << 60) | (self.asid.get() << 44) | self.root_ppn(),
        )
    }
    #[cfg(debug_assertions)]
    pub fn map_count(&self) -> usize {
        self.state.lock().index.len()
    }
    #[cfg(debug_assertions)]
    pub fn table_count(&self) -> usize {
        let state = self.state.lock();
        state.root.node_count()
            + state
                .retired_tables
                .iter()
                .map(TableNode::node_count)
                .sum::<usize>()
    }

    pub(crate) fn probe(&self, address: VirtAddr) -> Option<Arc<Map>> {
        self.state.lock().index.find(address.as_usize())
    }
    pub fn overlaps(&self, start: VirtAddr, size: usize) -> Result<bool, MapError> {
        if size == 0 || start.offset() != 0 || size & (PAGE_SIZE - 1) != 0 {
            return Err(MapError::NotAligned);
        }
        let end = start
            .as_usize()
            .checked_add(size - 1)
            .ok_or(MapError::NotAligned)?;
        let state = self.state.lock();
        Ok(state.index.first_overlap(start.as_usize(), end).is_some())
    }

    #[cfg(debug_assertions)]
    pub fn maps(&self) -> Result<Vec<Arc<Map>>, MapError> {
        let state = self.state.lock();
        let mut maps = Vec::new();
        maps.try_reserve(state.index.len())
            .map_err(|_| MapError::OutOfMemory)?;
        state.index.collect(&mut maps);
        Ok(maps)
    }

    pub fn has_pending_invalidation(&self) -> bool {
        !self.state.lock().invalidation.is_clean()
    }

    #[cfg(debug_assertions)]
    pub fn pending_maps(&self) -> Result<Vec<Arc<Map>>, MapError> {
        let state = self.state.lock();
        let mut maps = Vec::new();
        maps.try_reserve(state.pending_maps.len())
            .map_err(|_| MapError::OutOfMemory)?;
        maps.extend(state.pending_maps.iter().cloned());
        Ok(maps)
    }

    pub fn ranges(&self) -> Result<Vec<(Arc<Map>, MapRange)>, MapError> {
        let state = self.state.lock();
        let mut maps = Vec::new();
        maps.try_reserve(state.index.len())
            .map_err(|_| MapError::OutOfMemory)?;
        state.index.collect(&mut maps);
        let count = maps.iter().map(|map| map.state.lock().ranges.len()).sum();
        let mut ranges = Vec::new();
        ranges
            .try_reserve(count)
            .map_err(|_| MapError::OutOfMemory)?;
        for map in maps {
            ranges.extend(
                map.state
                    .lock()
                    .ranges
                    .iter()
                    .copied()
                    .map(|range| (map.clone(), range)),
            );
        }
        ranges.sort_unstable_by_key(|(_, range)| range.start);
        Ok(ranges)
    }

    pub fn map_frames(
        self: &Arc<Self>,
        start: VirtAddr,
        frames: Vec<Arc<Frame>>,
        ceiling: PteFlags,
        current: PteFlags,
    ) -> Result<Arc<Map>, MapError> {
        if frames.is_empty()
            || start.offset() != 0
            || !valid_leaf_flags(current)
            || !ceiling.contains(current & (PteFlags::R | PteFlags::W | PteFlags::X))
        {
            return Err(MapError::NotAligned);
        }
        let size = frames
            .len()
            .checked_mul(PAGE_SIZE)
            .ok_or(MapError::NotAligned)?;
        start
            .as_usize()
            .checked_add(size - 1)
            .ok_or(MapError::NotAligned)?;
        let map = Arc::try_new(Map::new(Arc::downgrade(self), start, size, ceiling)?)
            .map_err(|_| MapError::OutOfMemory)?;
        {
            let mut map_state = map.state.lock();
            map_state.ranges[0].current = current | PteFlags::V;
            map_state
                .frames
                .try_reserve(frames.len())
                .map_err(|_| MapError::OutOfMemory)?;
            for (index, frame) in frames.into_iter().enumerate() {
                map_state.frames.push((index, frame));
            }
        }
        let node = Box::try_new(IndexNode::new(map.clone())?).map_err(|_| MapError::OutOfMemory)?;
        let mut state = self.ready_state()?;
        let end = start
            .as_usize()
            .checked_add(size - 1)
            .ok_or(MapError::NotAligned)?;
        if state.index.first_overlap(start.as_usize(), end).is_some() {
            return Err(MapError::AlreadyMapped);
        }
        state
            .pending_maps
            .try_reserve(1)
            .map_err(|_| MapError::OutOfMemory)?;
        let map_state = map.state.lock();
        for run in map_state.runs(start, &map_state.frames) {
            let run = run?;
            state
                .root
                .prepare_range_geometry(run.start, run.physical, run.size, self.geometry)?;
        }
        state.pending_maps.push(map.clone());
        for run in map_state.runs(start, &map_state.frames) {
            let run = run?;
            state.root.map_range_prepared_geometry(
                run.start,
                run.physical,
                run.size,
                run.current,
                self.geometry,
            );
        }
        drop(map_state);
        state.index.insert(node);
        state.mark_dirty();
        drop(state);
        self.finish_invalidation()?;
        Ok(map)
    }

    pub fn map_lazy(
        self: &Arc<Self>,
        start: VirtAddr,
        size: usize,
        ceiling: PteFlags,
        current: PteFlags,
    ) -> Result<Arc<Map>, MapError> {
        if size == 0
            || start.offset() != 0
            || size & (PAGE_SIZE - 1) != 0
            || !valid_leaf_flags(current)
            || !ceiling.contains(current & (PteFlags::R | PteFlags::W | PteFlags::X))
        {
            return Err(MapError::NotAligned);
        }
        start
            .as_usize()
            .checked_add(size - 1)
            .ok_or(MapError::NotAligned)?;
        let map = Arc::try_new(Map::new(Arc::downgrade(self), start, size, ceiling)?)
            .map_err(|_| MapError::OutOfMemory)?;
        {
            let mut ms = map.state.lock();
            ms.ranges[0].current = current | PteFlags::V;
            ms.lazy = true;
        }
        let node = Box::try_new(IndexNode::new(map.clone())?).map_err(|_| MapError::OutOfMemory)?;
        let mut state = self.ready_state()?;
        let end = start
            .as_usize()
            .checked_add(size - 1)
            .ok_or(MapError::NotAligned)?;
        if state.index.first_overlap(start.as_usize(), end).is_some() {
            return Err(MapError::AlreadyMapped);
        }
        state.index.insert(node);
        Ok(map)
    }

    pub fn map_borrowed(
        self: &Arc<Self>,
        start: VirtAddr,
        region: Arc<BorrowedRegion>,
        offset: usize,
        size: usize,
        ceiling: PteFlags,
        current: PteFlags,
    ) -> Result<Arc<Map>, MapError> {
        if offset & (PAGE_SIZE - 1) != 0
            || size == 0
            || size & (PAGE_SIZE - 1) != 0
            || offset
                .checked_add(size)
                .is_none_or(|end| end > region.size())
            || !valid_leaf_flags(current)
            || !ceiling.contains(current & (PteFlags::R | PteFlags::W | PteFlags::X))
        {
            return Err(MapError::NotAligned);
        }
        start
            .as_usize()
            .checked_add(size - 1)
            .ok_or(MapError::NotAligned)?;
        let map = Arc::try_new(Map::new(Arc::downgrade(self), start, size, ceiling)?)
            .map_err(|_| MapError::OutOfMemory)?;
        map.state.lock().ranges[0].current = current | PteFlags::V;
        {
            let mut ms = map.state.lock();
            ms.borrowed = Some(region.clone());
            ms.borrowed_offset = offset;
        }
        let node = Box::try_new(IndexNode::new(map.clone())?).map_err(|_| MapError::OutOfMemory)?;
        let mut state = self.ready_state()?;
        let end = start
            .as_usize()
            .checked_add(size - 1)
            .ok_or(MapError::NotAligned)?;
        if state.index.first_overlap(start.as_usize(), end).is_some() {
            return Err(MapError::AlreadyMapped);
        }
        state
            .pending_maps
            .try_reserve(1)
            .map_err(|_| MapError::OutOfMemory)?;
        let physical_start = PhysAddr::from_raw(region.start.as_usize() + offset);
        state
            .root
            .prepare_range_geometry(start, physical_start, size, self.geometry)?;
        state.pending_maps.push(map.clone());
        state.root.map_range_prepared_geometry(
            start,
            physical_start,
            size,
            current | PteFlags::V,
            self.geometry,
        );
        state.index.insert(node);
        state.mark_dirty();
        drop(state);
        self.finish_invalidation()?;
        Ok(map)
    }

    pub fn materialize(
        &self,
        map: &Arc<Map>,
        start: VirtAddr,
        size: usize,
    ) -> Result<(), MapError> {
        if size == 0 || start.offset() != 0 || size & (PAGE_SIZE - 1) != 0 {
            return Err(MapError::NotAligned);
        }
        let end = start
            .as_usize()
            .checked_add(size - 1)
            .ok_or(MapError::NotAligned)?;
        let mut state = self.ready_state()?;
        ensure_map(self, map, &state)?;
        let mut ms = map.state.lock();
        if !ms.lazy || !ms.covers(start.as_usize(), end) {
            return Err(MapError::NoRegion);
        }
        let first = map.page_index(start);
        let last = map.page_index(VirtAddr::from_raw(end));
        let count = (first..=last).filter(|page| !ms.owns_page(*page)).count();
        if count == 0 {
            return Ok(());
        }
        let mut frames = Vec::new();
        frames
            .try_reserve(count)
            .map_err(|_| MapError::OutOfMemory)?;
        for page in first..=last {
            if !ms.owns_page(page) {
                frames.push((page, Frame::allocate()?));
            }
        }
        ms.frames
            .try_reserve(count)
            .map_err(|_| MapError::OutOfMemory)?;
        state
            .pending_maps
            .try_reserve(1)
            .map_err(|_| MapError::OutOfMemory)?;
        state.pending_maps.push(map.clone());
        for run in ms.runs(map.initial_start, &frames) {
            let run = run?;
            state
                .root
                .prepare_range_geometry(run.start, run.physical, run.size, self.geometry)?;
        }
        for run in ms.runs(map.initial_start, &frames) {
            let run = run?;
            state.root.map_range_prepared_geometry(
                run.start,
                run.physical,
                run.size,
                run.current | PteFlags::V,
                self.geometry,
            );
        }
        for (page, frame) in frames {
            ms.insert_frame(page, frame);
        }
        state.mark_dirty();
        drop(ms);
        drop(state);
        self.finish_invalidation()
    }

    pub fn derive_map(
        self: &Arc<Self>,
        source: &Arc<Map>,
        source_offset: usize,
        size: usize,
        start: VirtAddr,
        ceiling: PteFlags,
        current: PteFlags,
    ) -> Result<Arc<Map>, MapError> {
        if source_offset & (PAGE_SIZE - 1) != 0
            || size == 0
            || size & (PAGE_SIZE - 1) != 0
            || start.offset() != 0
            || !valid_leaf_flags(current)
            || !ceiling.contains(current & (PteFlags::R | PteFlags::W | PteFlags::X))
            || source_offset
                .checked_add(size)
                .is_none_or(|end| end > source.initial_size.get())
        {
            return Err(MapError::NotAligned);
        }
        start
            .as_usize()
            .checked_add(size - 1)
            .ok_or(MapError::NotAligned)?;
        let source_tree = source.tree.upgrade().ok_or(MapError::NoRegion)?;
        let src_lo = source.initial_start.as_usize() + source_offset;
        let src_hi = src_lo.checked_add(size - 1).ok_or(MapError::NotAligned)?;
        let (fragments, source_is_lazy) = {
            let ts = source_tree.state.lock();
            ensure_map(&source_tree, source, &ts)?;
            let ms = source.state.lock();
            let mut fragments = Vec::new();
            fragments
                .try_reserve(ms.ranges.len())
                .map_err(|_| MapError::OutOfMemory)?;
            for range in &ms.ranges {
                if let Some((lo, hi)) = range.intersection(src_lo, src_hi) {
                    fragments.push((VirtAddr::from_raw(lo), hi - lo + 1));
                }
            }
            if fragments.is_empty() {
                return Err(MapError::NoRegion);
            }
            (fragments, ms.lazy)
        };
        if source_is_lazy {
            for (lo, len) in &fragments {
                source_tree.materialize(source, *lo, *len)?;
            }
        }
        let (ranges, frames, borrowed, borrowed_offset) = {
            let ts = source_tree.state.lock();
            ensure_map(&source_tree, source, &ts)?;
            let ms = source.state.lock();
            let mut ranges = Vec::new();
            ranges
                .try_reserve(ms.ranges.len())
                .map_err(|_| MapError::OutOfMemory)?;
            for range in &ms.ranges {
                if let Some((lo, hi)) = range.intersection(src_lo, src_hi) {
                    ranges.push(MapRange {
                        start: start + (lo - src_lo),
                        size: NonZeroUsize::new(hi - lo + 1).unwrap(),
                        ceiling: ceiling & (PteFlags::R | PteFlags::W | PteFlags::X),
                        current: current | PteFlags::V,
                    });
                }
            }
            let frames = ms.frames_in_pages(
                source.page_index(source.initial_start + source_offset),
                source.page_index(source.initial_start + source_offset) + size / PAGE_SIZE,
            )?;
            (
                ranges,
                frames,
                ms.borrowed.clone(),
                ms.borrowed_offset + source_offset,
            )
        };
        let map = Arc::try_new(Map::new(Arc::downgrade(self), start, size, ceiling)?)
            .map_err(|_| MapError::OutOfMemory)?;
        {
            let mut target = map.state.lock();
            target.ranges = ranges;
            target.frames = frames;
            target.borrowed = borrowed;
            target.borrowed_offset = borrowed_offset;
            target.lazy = false;
        }
        let node = Box::try_new(IndexNode::new(map.clone())?).map_err(|_| MapError::OutOfMemory)?;
        let mut state = self.ready_state()?;
        for range in map.state.lock().ranges.iter() {
            if state
                .index
                .first_overlap(range.start.as_usize(), range.end())
                .is_some()
            {
                return Err(MapError::AlreadyMapped);
            }
        }
        let target = map.state.lock();
        let pte_count = target.pte_count();
        for run in target.runs(start, &target.frames) {
            let run = run?;
            state
                .root
                .prepare_range_geometry(run.start, run.physical, run.size, self.geometry)?;
        }
        if let Some(region) = &target.borrowed {
            for range in &target.ranges {
                let first_page = (range.start.as_usize() - start.as_usize()) / PAGE_SIZE;
                let pa = region.start + borrowed_offset + first_page * PAGE_SIZE;
                state.root.prepare_range_geometry(
                    range.start,
                    pa,
                    range.size.get(),
                    self.geometry,
                )?;
            }
        }
        state
            .pending_maps
            .try_reserve(1)
            .map_err(|_| MapError::OutOfMemory)?;
        if pte_count != 0 {
            state.pending_maps.push(map.clone());
        }
        for run in target.runs(start, &target.frames) {
            let run = run?;
            state.root.map_range_prepared_geometry(
                run.start,
                run.physical,
                run.size,
                run.current,
                self.geometry,
            );
        }
        if let Some(region) = &target.borrowed {
            for range in &target.ranges {
                let first_page = (range.start.as_usize() - start.as_usize()) / PAGE_SIZE;
                let pa = region.start + borrowed_offset + first_page * PAGE_SIZE;
                state.root.map_range_prepared_geometry(
                    range.start,
                    pa,
                    range.size.get(),
                    range.current,
                    self.geometry,
                );
            }
        }
        drop(target);
        state.index.insert(node);
        if pte_count != 0 {
            state.mark_dirty();
        }
        drop(state);
        if pte_count != 0 {
            self.finish_invalidation()?;
        }
        Ok(map)
    }

    pub fn narrow(
        &self,
        map: &Arc<Map>,
        start: VirtAddr,
        size: usize,
        ceiling: PteFlags,
    ) -> Result<(), MapError> {
        if size == 0 || start.offset() != 0 || size & (PAGE_SIZE - 1) != 0 {
            return Err(MapError::NotAligned);
        }
        let end = start
            .as_usize()
            .checked_add(size - 1)
            .ok_or(MapError::NotAligned)?;
        let rwx = PteFlags::R | PteFlags::W | PteFlags::X;
        let ceiling = ceiling & rwx;
        let mut state = self.ready_state()?;
        ensure_map(self, map, &state)?;
        let mut ms = map.state.lock();
        if !ms.covers(start.as_usize(), end) {
            return Err(MapError::NoRegion);
        }
        let next = MapRange::narrow(&ms.ranges, start.as_usize(), end, ceiling)?;
        let mut updates = Vec::new();
        let first = map.page_index(start);
        let pages = size / PAGE_SIZE;
        updates
            .try_reserve(pages)
            .map_err(|_| MapError::OutOfMemory)?;
        for page in first..first + pages {
            let va = map.initial_start + page * PAGE_SIZE;
            let Some(range) = next.iter().find(|range| range.contains(va.as_usize())) else {
                return Err(MapError::NoRegion);
            };
            if ms.borrowed.is_some() || ms.owns_page(page) {
                state.root.prepare_geometry(va, PAGE_SIZE, self.geometry)?;
                state.root.walk_ref_geometry(va, self.geometry)?;
                updates.push((va, range.current));
            }
        }
        state
            .pending_maps
            .try_reserve(1)
            .map_err(|_| MapError::OutOfMemory)?;
        state.pending_maps.push(map.clone());
        for (va, flags) in updates {
            state
                .root
                .set_permissions_prepared_geometry(va, flags, self.geometry);
        }
        ms.ranges = next;
        drop(ms);
        state.index.refresh(map);
        state.mark_dirty();
        drop(state);
        self.finish_invalidation()
    }

    pub fn find(&self, address: VirtAddr) -> Option<Arc<Map>> {
        let state = self.ready_state().ok()?;
        state.index.find(address.as_usize())
    }

    #[cfg(debug_assertions)]
    pub(crate) fn leaf_span(&self, address: VirtAddr) -> Option<usize> {
        let state = self.ready_state().ok()?;
        state.index.find(address.as_usize())?;
        state.root.leaf_span_geometry(address, self.geometry)
    }

    pub fn check_access(&self, start: VirtAddr, size: usize, need: PteFlags) -> bool {
        self.access_range(start, size, need, None)
    }

    pub fn copy_from(&self, start: VirtAddr, dst: &mut [u8], need: PteFlags) -> bool {
        self.access_range(start, dst.len(), need, Some((dst.as_mut_ptr(), false)))
    }

    pub fn copy_to(&self, start: VirtAddr, src: &[u8], need: PteFlags) -> bool {
        self.access_range(
            start,
            src.len(),
            need,
            Some((src.as_ptr() as *mut u8, true)),
        )
    }

    fn access_range(
        &self,
        start: VirtAddr,
        size: usize,
        need: PteFlags,
        buffer: Option<(*mut u8, bool)>,
    ) -> bool {
        if start.as_usize().checked_add(size).is_none() {
            return false;
        }
        let Ok(state) = self.ready_state() else {
            return false;
        };
        let mut offset = 0;
        while offset < size {
            let at = start.as_usize() + offset;
            let Some(map) = state.index.find(at) else {
                return false;
            };
            let map_state = map.state.lock();
            let Some(range) = map_state.range_at(at) else {
                return false;
            };
            let page = VirtAddr::from_raw(at & !(PAGE_SIZE - 1));
            let Ok((_, hardware)) = state.root.walk_ref_geometry(page, self.geometry) else {
                return false;
            };
            if !range.current.contains(need)
                || !hardware.contains(need)
                || hardware.contains(PteFlags::G)
            {
                return false;
            }
            let count = (PAGE_SIZE - (at & (PAGE_SIZE - 1))).min(size - offset);
            offset += count;
        }
        let Some((buffer, write_to_user)) = buffer else {
            return true;
        };
        offset = 0;
        while offset < size {
            let at = start.as_usize() + offset;
            let Some(map) = state.index.find(at) else {
                unreachable!("Tree held across access validation and copy")
            };
            let _map_state = map.state.lock();
            let page = VirtAddr::from_raw(at & !(PAGE_SIZE - 1));
            let (physical, _) = state
                .root
                .walk_ref_geometry(page, self.geometry)
                .expect("validated page table leaf");
            let within = at & (PAGE_SIZE - 1);
            let count = (PAGE_SIZE - within).min(size - offset);
            let physical = physical.as_usize() as *mut u8;
            unsafe {
                if write_to_user {
                    core::ptr::copy(buffer.add(offset), physical.add(within), count);
                } else {
                    core::ptr::copy(physical.add(within), buffer.add(offset), count);
                }
            }
            offset += count;
        }
        true
    }

    pub fn translate(&self, address: VirtAddr) -> Option<(PhysAddr, PteFlags)> {
        let state = self.ready_state().ok()?;
        state.index.find(address.as_usize())?;
        let (pa, flags) = state.root.walk_ref_geometry(address, self.geometry).ok()?;
        Some((pa + address.offset(), flags))
    }

    pub fn protect(
        &self,
        map: &Arc<Map>,
        start: VirtAddr,
        size: usize,
        flags: PteFlags,
    ) -> Result<(), MapError> {
        if size == 0
            || start.offset() != 0
            || size & (PAGE_SIZE - 1) != 0
            || !valid_leaf_flags(flags)
        {
            return Err(MapError::NotAligned);
        }
        let end = start
            .as_usize()
            .checked_add(size - 1)
            .ok_or(MapError::NotAligned)?;
        let rwx = PteFlags::R | PteFlags::W | PteFlags::X;
        let mut state = self.ready_state()?;
        ensure_map(self, map, &state)?;
        let mut ms = map.state.lock();
        if !ms.covers(start.as_usize(), end)
            || ms
                .ranges
                .iter()
                .filter(|range| range.overlaps(start.as_usize(), end))
                .any(|range| !range.ceiling.contains(flags & rwx))
        {
            return Err(MapError::WidenDenied);
        }
        let changed = MapRange::split(&ms.ranges, start.as_usize(), end, Some(flags))?;
        let first = map.page_index(start);
        let pages = size / PAGE_SIZE;
        let mut updates = Vec::new();
        updates
            .try_reserve(pages)
            .map_err(|_| MapError::OutOfMemory)?;
        for page in first..first + pages {
            let va = map.initial_start + page * PAGE_SIZE;
            if ms.borrowed.is_some() || ms.owns_page(page) {
                state.root.prepare_geometry(va, PAGE_SIZE, self.geometry)?;
                state.root.walk_ref_geometry(va, self.geometry)?;
                updates.push(va);
            }
        }
        state
            .pending_maps
            .try_reserve(1)
            .map_err(|_| MapError::OutOfMemory)?;
        state.pending_maps.push(map.clone());
        for va in updates {
            state
                .root
                .set_permissions_prepared_geometry(va, flags, self.geometry);
        }
        ms.ranges = changed;
        drop(ms);
        state.index.refresh(map);
        state.mark_dirty();
        drop(state);
        self.finish_invalidation()
    }

    pub fn unmap(&self, map: &Arc<Map>, start: VirtAddr, size: usize) -> Result<(), MapError> {
        if size == 0 || start.offset() != 0 || !size.is_multiple_of(PAGE_SIZE) {
            return Err(MapError::NotAligned);
        }
        let end = start
            .as_usize()
            .checked_add(size - 1)
            .ok_or(MapError::NotAligned)?;
        let mut state = self.ready_state()?;
        ensure_map(self, map, &state)?;
        let mut map_state = map.state.lock();
        if !map_state.covers(start.as_usize(), end) {
            return Err(MapError::NoRegion);
        }
        let ranges = MapRange::split(&map_state.ranges, start.as_usize(), end, None)?;
        let next_node = if ranges.is_empty() {
            None
        } else {
            Some(
                Box::try_new(IndexNode::with_ranges(map.clone(), &ranges)?)
                    .map_err(|_| MapError::OutOfMemory)?,
            )
        };
        let first_page = map.page_index(start);
        let last_page = map.page_index(VirtAddr::from_raw(end));
        let removed = map_state
            .frames
            .iter()
            .filter(|(page, _)| *page >= first_page && *page <= last_page)
            .count();
        state
            .retired_frames
            .try_reserve(removed)
            .map_err(|_| MapError::OutOfMemory)?;
        state
            .pending_maps
            .try_reserve(1)
            .map_err(|_| MapError::OutOfMemory)?;
        {
            let TreeState {
                root,
                retired_tables,
                ..
            } = &mut *state;
            root.prepare_geometry(start, size, self.geometry)?;
            root.reserve_retired_tables(retired_tables)?;
            root.unmap_keep_tables_geometry(start, size, self.geometry)?;
            root.reclaim_empty_tables(retired_tables);
        }
        state.pending_maps.push(map.clone());
        for index in (0..map_state.frames.len()).rev() {
            let page = map_state.frames[index].0;
            if page >= first_page && page <= last_page {
                let (_, frame) = map_state.frames.remove(index);
                state.retired_frames.push(frame);
            }
        }
        map_state.ranges = ranges;
        drop(map_state);
        let removed = state.index.remove(map);
        debug_assert!(removed, "live map must have an index node");
        if let Some(node) = next_node {
            state.index.insert(node);
        }
        state.mark_dirty();
        drop(state);
        self.finish_invalidation()
    }

    pub fn retry_invalidation(&self) -> Result<(), MapError> {
        if self.state.lock().invalidation.is_clean() {
            return Ok(());
        }
        self.finish_invalidation()
    }

    fn finish_invalidation(&self) -> Result<(), MapError> {
        loop {
            let mut state = self.state.lock();
            if state.invalidation.is_clean() {
                return Ok(());
            }
            if state.invalidation.is_fencing() {
                drop(state);
                while self.state.lock().invalidation.is_fencing() {
                    core::hint::spin_loop();
                }
                continue;
            }
            state.fence();
            drop(state);

            let result = asid::shootdown(self.asid).map_err(|_| MapError::Shootdown);
            let retired_tables;
            let retired_frames;
            let pending_maps;
            {
                let mut state = self.state.lock();
                if let Err(error) = result {
                    state.fail();
                    return Err(error);
                }
                state.settle();
                retired_tables = core::mem::take(&mut state.retired_tables);
                retired_frames = core::mem::take(&mut state.retired_frames);
                pending_maps = core::mem::take(&mut state.pending_maps);
            }
            drop(retired_tables);
            drop(retired_frames);
            drop(pending_maps);
            return Ok(());
        }
    }

    pub fn close(&self) -> Result<(), MapError> {
        if self.asid.is_kernel() {
            return Err(MapError::Pinned);
        }
        let mut state = loop {
            let state = self.state.lock();
            if state.phase.is_closed() {
                return Ok(());
            }
            if state.invalidation.is_clean() {
                break state;
            }
            drop(state);
            self.finish_invalidation()?;
        };
        let mut maps = Vec::new();
        maps.try_reserve(state.index.len())
            .map_err(|_| MapError::OutOfMemory)?;
        state.index.collect(&mut maps);
        let frame_count = maps.iter().map(|map| map.state.lock().frames.len()).sum();
        state
            .retired_frames
            .try_reserve(frame_count)
            .map_err(|_| MapError::OutOfMemory)?;
        state
            .pending_maps
            .try_reserve(maps.len())
            .map_err(|_| MapError::OutOfMemory)?;
        {
            let TreeState {
                root,
                retired_tables,
                ..
            } = &mut *state;
            for map in &maps {
                for range in &map.state.lock().ranges {
                    root.prepare_geometry(range.start, range.size.get(), self.geometry)?;
                }
            }
            root.reserve_retired_tables(retired_tables)?;
            for map in &maps {
                for range in &map.state.lock().ranges {
                    root.unmap_keep_tables_geometry(range.start, range.size.get(), self.geometry)?;
                }
            }
            root.reclaim_empty_tables(retired_tables);
        }
        for map in maps {
            state.pending_maps.push(map.clone());
            let mut map_state = map.state.lock();
            for (_, frame) in map_state.frames.drain(..) {
                state.retired_frames.push(frame);
            }
            map_state.ranges.clear();
        }
        state.index.clear();
        state.mark_dirty();
        state.begin_close();
        drop(state);
        self.finish_invalidation()
    }

    #[cfg(debug_assertions)]
    pub(crate) fn audit(&self) {
        let state = self.state.lock();
        state.index.audit();
        state.index.audit_maps(self, &state.root, self.geometry);
        state.root.visit_leaves(
            (self.geometry.levels - 1) as usize,
            0,
            self.geometry,
            &mut |start, physical, flags, size| {
                let end = start
                    .as_usize()
                    .checked_add(size - 1)
                    .expect("leaf virtual interval is representable");
                let map = state
                    .index
                    .find(start.as_usize())
                    .expect("hardware leaf is absent from the Tree index");
                let end_map = state
                    .index
                    .find(end)
                    .expect("hardware leaf extends beyond the Tree index");
                assert!(
                    Arc::ptr_eq(&map, &end_map),
                    "hardware leaf crosses Map identity"
                );
                let map_state = map.state.lock();
                assert!(
                    map_state.covers(start.as_usize(), end),
                    "hardware leaf crosses an unmapped Map range"
                );
                let mut expected_flags: Option<PteFlags> = None;
                for range in map_state
                    .ranges
                    .iter()
                    .filter(|range| range.overlaps(start.as_usize(), end))
                {
                    let normalized = range.current
                        & (PteFlags::V
                            | PteFlags::R
                            | PteFlags::W
                            | PteFlags::X
                            | PteFlags::U
                            | PteFlags::G);
                    if let Some(expected) = expected_flags {
                        assert_eq!(
                            normalized.bits(),
                            expected.bits(),
                            "hardware leaf crosses permission state"
                        );
                    } else {
                        expected_flags = Some(normalized);
                    }
                }
                assert_eq!(
                    (flags
                        & (PteFlags::V
                            | PteFlags::R
                            | PteFlags::W
                            | PteFlags::X
                            | PteFlags::U
                            | PteFlags::G))
                        .bits(),
                    expected_flags
                        .expect("hardware leaf belongs to a live range")
                        .bits(),
                    "hardware PTE permissions differ from Map state"
                );
                if let Some(region) = &map_state.borrowed {
                    let offset = start
                        .as_usize()
                        .checked_sub(map.initial_start.as_usize())
                        .expect("borrowed leaf precedes Map start");
                    let expected = region
                        .start
                        .as_usize()
                        .checked_add(map_state.borrowed_offset)
                        .and_then(|base| base.checked_add(offset))
                        .expect("borrowed physical interval is representable");
                    assert_eq!(
                        physical.as_usize(),
                        expected,
                        "borrowed PTE physical address differs"
                    );
                } else {
                    let first_page = map.page_index(start);
                    let pages = size / PAGE_SIZE;
                    let first = map_state
                        .frames
                        .partition_point(|(page, _)| *page < first_page);
                    let last_page = first_page + pages;
                    let last = map_state
                        .frames
                        .partition_point(|(page, _)| *page < last_page);
                    assert_eq!(
                        last - first,
                        pages,
                        "hardware leaf has pages without owned Frames"
                    );
                    for (offset, (page, frame)) in map_state.frames[first..last].iter().enumerate()
                    {
                        assert_eq!(
                            *page,
                            first_page + offset,
                            "owned Frame index gap under hardware leaf"
                        );
                        assert_eq!(
                            frame.physical_address(),
                            physical.as_usize() + offset * PAGE_SIZE,
                            "owned Frame physical address differs from hardware PTE"
                        );
                    }
                }
            },
        );
    }

    fn is_closed(&self) -> bool {
        self.state.lock().phase.is_closed()
    }
}

fn ensure_map(tree: &Tree, map: &Arc<Map>, state: &TreeState) -> Result<(), MapError> {
    let owner = map.tree.upgrade().ok_or(MapError::NoRegion)?;
    if state.phase.is_closed()
        || !core::ptr::eq(Arc::as_ptr(&owner), tree)
        || map.state.lock().ranges.is_empty()
    {
        return Err(MapError::NoRegion);
    }
    Ok(())
}

impl Drop for Tree {
    fn drop(&mut self) {
        if !self.published.load(Ordering::Acquire) {
            if !self.asid.is_kernel() {
                Asid::release_unpublished(self.asid);
            }
        } else if self.asid.is_kernel() {
            return;
        } else if !self.asid_released.load(Ordering::Acquire) {
            if self.close().is_err() || asid::deallocate(self.asid).is_err() {
                return;
            }
            self.asid_released.store(true, Ordering::Release);
        }
        unsafe { ManuallyDrop::drop(&mut self.state) };
    }
}
