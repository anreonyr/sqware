use alloc::boxed::Box;
use alloc::sync::{Arc, Weak};
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

#[derive(Debug, Clone, Copy)]
pub struct MapRange {
    pub start: VirtAddr,
    pub size: NonZeroUsize,
    pub ceiling: PteFlags,
    pub current: PteFlags,
}

impl MapRange {
    pub fn end(self) -> usize {
        self.start.as_usize().saturating_add(self.size.get() - 1)
    }
}

pub struct Map {
    tree: Weak<Tree>,
    initial_start: VirtAddr,
    initial_size: NonZeroUsize,
    state: SpinLock<MapState>,
}

impl core::fmt::Debug for Map {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Map")
            .field("initial_start", &self.initial_start)
            .field("initial_size", &self.initial_size)
            .finish_non_exhaustive()
    }
}

struct MapState {
    ranges: Vec<MapRange>,
    frames: Vec<(usize, Arc<Frame>)>,
    borrowed: Option<Arc<BorrowedRegion>>,
    borrowed_offset: usize,
    lazy: bool,
}

pub struct BorrowedRegion {
    start: PhysAddr,
    size: NonZeroUsize,
}

impl BorrowedRegion {
    pub unsafe fn new(start: PhysAddr, size: usize) -> Result<Arc<Self>, MapError> {
        if !start.is_aligned() || size == 0 || size & (PAGE_SIZE - 1) != 0 {
            return Err(MapError::NotAligned);
        }
        start
            .as_usize()
            .checked_add(size - 1)
            .ok_or(MapError::NotAligned)?;
        let size = NonZeroUsize::new(size).ok_or(MapError::NotAligned)?;
        Arc::try_new(Self { start, size }).map_err(|_| MapError::OutOfMemory)
    }

    pub fn size(&self) -> usize {
        self.size.get()
    }
}

impl Map {
    fn new(
        tree: Weak<Tree>,
        start: VirtAddr,
        size: usize,
        ceiling: PteFlags,
    ) -> Result<Self, MapError> {
        if size == 0
            || start.offset() != 0
            || !size.is_multiple_of(PAGE_SIZE)
            || start.as_usize().checked_add(size - 1).is_none()
        {
            return Err(MapError::NotAligned);
        }
        let size = NonZeroUsize::new(size).ok_or(MapError::NotAligned)?;
        let mut ranges = Vec::new();
        ranges.try_reserve(1).map_err(|_| MapError::OutOfMemory)?;
        ranges.push(MapRange {
            start,
            size,
            ceiling: ceiling & (PteFlags::R | PteFlags::W | PteFlags::X),
            current: ceiling,
        });
        Ok(Self {
            tree,
            initial_start: start,
            initial_size: size,
            state: SpinLock::new_level(
                Level::Map,
                MapState {
                    ranges,
                    frames: Vec::new(),
                    borrowed: None,
                    borrowed_offset: 0,
                    lazy: false,
                },
            ),
        })
    }

    pub fn tree(&self) -> Option<Arc<Tree>> {
        let tree = self.tree.upgrade()?;
        (!tree.is_closed()).then_some(tree)
    }

    pub fn initial_range(&self) -> (VirtAddr, usize) {
        (self.initial_start, self.initial_size.get())
    }

    pub fn ranges(&self) -> Result<Vec<MapRange>, MapError> {
        let state = self.state.lock();
        let mut ranges = Vec::new();
        ranges
            .try_reserve(state.ranges.len())
            .map_err(|_| MapError::OutOfMemory)?;
        ranges.extend_from_slice(&state.ranges);
        Ok(ranges)
    }

    pub fn frame_count(&self) -> usize {
        self.state.lock().frames.len()
    }

    #[cfg(debug_assertions)]
    pub fn frames_in(
        &self,
        start: VirtAddr,
        size: usize,
    ) -> Result<Vec<(usize, Arc<Frame>)>, MapError> {
        if size == 0 || start.offset() != 0 || size & (PAGE_SIZE - 1) != 0 {
            return Err(MapError::NotAligned);
        }
        let end = start
            .as_usize()
            .checked_add(size - 1)
            .ok_or(MapError::NotAligned)?;
        let state = self.state.lock();
        if !ranges_cover(&state.ranges, start.as_usize(), end) || state.borrowed.is_some() {
            return Err(MapError::NoRegion);
        }
        let first = start
            .as_usize()
            .checked_sub(self.initial_start.as_usize())
            .ok_or(MapError::NoRegion)?
            / PAGE_SIZE;
        let pages = size / PAGE_SIZE;
        let last = first.checked_add(pages - 1).ok_or(MapError::NotAligned)?;
        let count = state
            .frames
            .iter()
            .filter(|(page, _)| *page >= first && *page <= last)
            .count();
        if count != pages {
            return Err(MapError::NoRegion);
        }
        let mut out = Vec::new();
        out.try_reserve_exact(pages)
            .map_err(|_| MapError::OutOfMemory)?;
        out.extend(
            state
                .frames
                .iter()
                .filter(|(page, _)| *page >= first && *page <= last)
                .map(|(page, frame)| (*page, frame.clone())),
        );
        Ok(out)
    }

    pub fn is_live(&self) -> bool {
        self.tree().is_some() && !self.state.lock().ranges.is_empty()
    }

    fn overlaps(&self, lo: usize, hi: usize) -> bool {
        self.state
            .lock()
            .ranges
            .iter()
            .any(|range| range.start.as_usize() <= hi && range.end() >= lo)
    }
}

struct IndexNode {
    map: Arc<Map>,
    left: Option<Box<IndexNode>>,
    right: Option<Box<IndexNode>>,
    start: VirtAddr,
    height: u8,
    max_end: usize,
}

impl IndexNode {
    fn new(map: Arc<Map>) -> Result<Self, MapError> {
        let state = map.state.lock();
        let start = state
            .ranges
            .iter()
            .map(|r| r.start)
            .min()
            .ok_or(MapError::NoRegion)?;
        let max_end = state
            .ranges
            .iter()
            .map(|r| r.end())
            .max()
            .ok_or(MapError::NoRegion)?;
        Ok(Self {
            map: map.clone(),
            left: None,
            right: None,
            start,
            height: 1,
            max_end,
        })
    }

    fn with_ranges(map: Arc<Map>, ranges: &[MapRange]) -> Result<Self, MapError> {
        let start = ranges
            .iter()
            .map(|r| r.start)
            .min()
            .ok_or(MapError::NoRegion)?;
        let max_end = ranges
            .iter()
            .map(|r| r.end())
            .max()
            .ok_or(MapError::NoRegion)?;
        Ok(Self {
            map,
            left: None,
            right: None,
            start,
            height: 1,
            max_end,
        })
    }
}

struct TreeState {
    root: TableNode,
    index: Option<Box<IndexNode>>,
    len: usize,
    retired_frames: Vec<Arc<Frame>>,
    retired_tables: Vec<TableNode>,
    pending: bool,
    fencing: bool,
    failed: bool,
    pending_maps: Vec<Arc<Map>>,
    close_pending: bool,
    closed: bool,
}

pub struct Tree {
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
            if state.closed {
                return Err(MapError::NoRegion);
            }
            if !state.pending {
                return Ok(state);
            }
            if state.failed && !state.fencing {
                return Err(MapError::Shootdown);
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
            state: ManuallyDrop::new(SpinLock::new_level(
                Level::Tree,
                TreeState {
                    root,
                    index: None,
                    len: 0,
                    retired_frames: Vec::new(),
                    retired_tables: Vec::new(),
                    pending: false,
                    fencing: false,
                    failed: false,
                    pending_maps: Vec::new(),
                    close_pending: false,
                    closed: false,
                },
            )),
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
            state: ManuallyDrop::new(SpinLock::new_level(
                Level::Tree,
                TreeState {
                    root,
                    index: None,
                    len: 0,
                    retired_frames: Vec::new(),
                    retired_tables: Vec::new(),
                    pending: false,
                    fencing: false,
                    failed: false,
                    pending_maps: Vec::new(),
                    close_pending: false,
                    closed: false,
                },
            )),
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
        self.state.lock().len
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
        find_map(self.state.lock().index.as_deref(), address.as_usize())
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
        Ok(first_overlap(state.index.as_deref(), start.as_usize(), end).is_some())
    }

    #[cfg(debug_assertions)]
    pub fn maps(&self) -> Result<Vec<Arc<Map>>, MapError> {
        let state = self.state.lock();
        let mut maps = Vec::new();
        maps.try_reserve(state.len)
            .map_err(|_| MapError::OutOfMemory)?;
        collect_maps(state.index.as_deref(), &mut maps);
        Ok(maps)
    }

    pub fn has_pending_invalidation(&self) -> bool {
        self.state.lock().pending
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
        maps.try_reserve(state.len)
            .map_err(|_| MapError::OutOfMemory)?;
        collect_maps(state.index.as_deref(), &mut maps);
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
        if first_overlap(state.index.as_deref(), start.as_usize(), end).is_some() {
            return Err(MapError::AlreadyMapped);
        }
        state
            .pending_maps
            .try_reserve(1)
            .map_err(|_| MapError::OutOfMemory)?;
        let map_state = map.state.lock();
        let mut first = 0;
        while first < map_state.frames.len() {
            let mut last = first + 1;
            while last < map_state.frames.len()
                && map_state.frames[last].0 == map_state.frames[last - 1].0 + 1
                && map_state.frames[last].1.physical_address()
                    == map_state.frames[last - 1].1.physical_address() + PAGE_SIZE
            {
                last += 1;
            }
            state.root.prepare_range_geometry(
                start + map_state.frames[first].0 * PAGE_SIZE,
                PhysAddr::from_raw(map_state.frames[first].1.physical_address()),
                (last - first) * PAGE_SIZE,
                self.geometry,
            )?;
            first = last;
        }
        state.pending_maps.push(map.clone());
        let mut first = 0;
        while first < map_state.frames.len() {
            let mut last = first + 1;
            while last < map_state.frames.len()
                && map_state.frames[last].0 == map_state.frames[last - 1].0 + 1
                && map_state.frames[last].1.physical_address()
                    == map_state.frames[last - 1].1.physical_address() + PAGE_SIZE
            {
                last += 1;
            }
            state.root.map_range_prepared_geometry(
                start + map_state.frames[first].0 * PAGE_SIZE,
                PhysAddr::from_raw(map_state.frames[first].1.physical_address()),
                (last - first) * PAGE_SIZE,
                current | PteFlags::V,
                self.geometry,
            );
            first = last;
        }
        drop(map_state);
        state.index = Some(insert(state.index.take(), node));
        state.len += 1;
        state.pending = true;
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
        if first_overlap(state.index.as_deref(), start.as_usize(), end).is_some() {
            return Err(MapError::AlreadyMapped);
        }
        state.index = Some(insert(state.index.take(), node));
        state.len += 1;
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
        if first_overlap(state.index.as_deref(), start.as_usize(), end).is_some() {
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
        state.index = Some(insert(state.index.take(), node));
        state.len += 1;
        state.pending = true;
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
        if !ms.lazy || !ranges_cover(&ms.ranges, start.as_usize(), end) {
            return Err(MapError::NoRegion);
        }
        let first = (start.as_usize() - map.initial_start.as_usize()) / PAGE_SIZE;
        let last = (end - map.initial_start.as_usize()) / PAGE_SIZE;
        let count = (first..=last)
            .filter(|page| !ms.frames.iter().any(|(p, _)| p == page))
            .count();
        if count == 0 {
            return Ok(());
        }
        let mut frames = Vec::new();
        frames
            .try_reserve(count)
            .map_err(|_| MapError::OutOfMemory)?;
        for page in first..=last {
            if !ms.frames.iter().any(|(p, _)| *p == page) {
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
        let mut first_run = 0;
        while first_run < frames.len() {
            let mut end_run = first_run + 1;
            let first_va = map.initial_start + frames[first_run].0 * PAGE_SIZE;
            ms.ranges
                .iter()
                .find(|r| {
                    r.start.as_usize() <= first_va.as_usize() && r.end() >= first_va.as_usize()
                })
                .ok_or(MapError::NoRegion)?;
            while end_run < frames.len() {
                let prev_va = map.initial_start + frames[end_run - 1].0 * PAGE_SIZE;
                let va = map.initial_start + frames[end_run].0 * PAGE_SIZE;
                let prev_flags = ms
                    .ranges
                    .iter()
                    .find(|r| {
                        r.start.as_usize() <= prev_va.as_usize() && r.end() >= prev_va.as_usize()
                    })
                    .ok_or(MapError::NoRegion)?
                    .current;
                let next_flags = ms
                    .ranges
                    .iter()
                    .find(|r| r.start.as_usize() <= va.as_usize() && r.end() >= va.as_usize())
                    .ok_or(MapError::NoRegion)?
                    .current;
                if frames[end_run].0 != frames[end_run - 1].0 + 1
                    || frames[end_run].1.physical_address()
                        != frames[end_run - 1].1.physical_address() + PAGE_SIZE
                    || prev_flags.bits() != next_flags.bits()
                {
                    break;
                }
                end_run += 1;
            }
            state.root.prepare_range_geometry(
                first_va,
                PhysAddr::from_raw(frames[first_run].1.physical_address()),
                (end_run - first_run) * PAGE_SIZE,
                self.geometry,
            )?;
            first_run = end_run;
        }
        let mut first_run = 0;
        while first_run < frames.len() {
            let mut end_run = first_run + 1;
            let first_va = map.initial_start + frames[first_run].0 * PAGE_SIZE;
            let flags = ms
                .ranges
                .iter()
                .find(|r| {
                    r.start.as_usize() <= first_va.as_usize() && r.end() >= first_va.as_usize()
                })
                .ok_or(MapError::NoRegion)?
                .current
                | PteFlags::V;
            while end_run < frames.len() {
                let prev_va = map.initial_start + frames[end_run - 1].0 * PAGE_SIZE;
                let va = map.initial_start + frames[end_run].0 * PAGE_SIZE;
                let prev_flags = ms
                    .ranges
                    .iter()
                    .find(|r| {
                        r.start.as_usize() <= prev_va.as_usize() && r.end() >= prev_va.as_usize()
                    })
                    .ok_or(MapError::NoRegion)?
                    .current;
                let next_flags = ms
                    .ranges
                    .iter()
                    .find(|r| r.start.as_usize() <= va.as_usize() && r.end() >= va.as_usize())
                    .ok_or(MapError::NoRegion)?
                    .current;
                if frames[end_run].0 != frames[end_run - 1].0 + 1
                    || frames[end_run].1.physical_address()
                        != frames[end_run - 1].1.physical_address() + PAGE_SIZE
                    || prev_flags.bits() != next_flags.bits()
                {
                    break;
                }
                end_run += 1;
            }
            state.root.map_range_prepared_geometry(
                first_va,
                PhysAddr::from_raw(frames[first_run].1.physical_address()),
                (end_run - first_run) * PAGE_SIZE,
                flags,
                self.geometry,
            );
            first_run = end_run;
        }
        for (page, frame) in frames {
            let at = ms.frames.partition_point(|(p, _)| *p < page);
            ms.frames.insert(at, (page, frame));
        }
        state.pending = true;
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
                let lo = range.start.as_usize().max(src_lo);
                let hi = range.end().min(src_hi);
                if lo <= hi {
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
                let lo = range.start.as_usize().max(src_lo);
                let hi = range.end().min(src_hi);
                if lo <= hi {
                    ranges.push(MapRange {
                        start: start + (lo - src_lo),
                        size: NonZeroUsize::new(hi - lo + 1).unwrap(),
                        ceiling: ceiling & (PteFlags::R | PteFlags::W | PteFlags::X),
                        current: current | PteFlags::V,
                    });
                }
            }
            let first_page = source_offset / PAGE_SIZE;
            let last_page = first_page + size / PAGE_SIZE;
            let mut frames = Vec::new();
            frames
                .try_reserve(
                    ms.frames
                        .iter()
                        .filter(|(p, _)| *p >= first_page && *p < last_page)
                        .count(),
                )
                .map_err(|_| MapError::OutOfMemory)?;
            frames.extend(
                ms.frames
                    .iter()
                    .filter(|(p, _)| *p >= first_page && *p < last_page)
                    .map(|(p, frame)| (*p - first_page, frame.clone())),
            );
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
            if first_overlap(state.index.as_deref(), range.start.as_usize(), range.end()).is_some()
            {
                return Err(MapError::AlreadyMapped);
            }
        }
        let target = map.state.lock();
        let pte_count = target.frames.len()
            + target.borrowed.as_ref().map_or(0, |_| {
                target.ranges.iter().map(|r| r.size.get() / PAGE_SIZE).sum()
            });
        let mut first = 0;
        while first < target.frames.len() {
            let mut last = first + 1;
            while last < target.frames.len()
                && target.frames[last].0 == target.frames[last - 1].0 + 1
                && target.frames[last].1.physical_address()
                    == target.frames[last - 1].1.physical_address() + PAGE_SIZE
                && target
                    .ranges
                    .iter()
                    .find(|r| {
                        r.start.as_usize() <= start.as_usize() + target.frames[last].0 * PAGE_SIZE
                            && r.end() >= start.as_usize() + target.frames[last].0 * PAGE_SIZE
                    })
                    .map(|r| r.current.bits())
                    == target
                        .ranges
                        .iter()
                        .find(|r| {
                            r.start.as_usize()
                                <= start.as_usize() + target.frames[last - 1].0 * PAGE_SIZE
                                && r.end()
                                    >= start.as_usize() + target.frames[last - 1].0 * PAGE_SIZE
                        })
                        .map(|r| r.current.bits())
            {
                last += 1;
            }
            state.root.prepare_range_geometry(
                start + target.frames[first].0 * PAGE_SIZE,
                PhysAddr::from_raw(target.frames[first].1.physical_address()),
                (last - first) * PAGE_SIZE,
                self.geometry,
            )?;
            first = last;
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
        let mut first = 0;
        while first < target.frames.len() {
            let mut last = first + 1;
            while last < target.frames.len()
                && target.frames[last].0 == target.frames[last - 1].0 + 1
                && target.frames[last].1.physical_address()
                    == target.frames[last - 1].1.physical_address() + PAGE_SIZE
                && target
                    .ranges
                    .iter()
                    .find(|r| {
                        r.start.as_usize() <= start.as_usize() + target.frames[last].0 * PAGE_SIZE
                            && r.end() >= start.as_usize() + target.frames[last].0 * PAGE_SIZE
                    })
                    .map(|r| r.current.bits())
                    == target
                        .ranges
                        .iter()
                        .find(|r| {
                            r.start.as_usize()
                                <= start.as_usize() + target.frames[last - 1].0 * PAGE_SIZE
                                && r.end()
                                    >= start.as_usize() + target.frames[last - 1].0 * PAGE_SIZE
                        })
                        .map(|r| r.current.bits())
            {
                last += 1;
            }
            let va = start + target.frames[first].0 * PAGE_SIZE;
            let flags = target
                .ranges
                .iter()
                .find(|r| r.start.as_usize() <= va.as_usize() && r.end() >= va.as_usize())
                .ok_or(MapError::NoRegion)?
                .current;
            state.root.map_range_prepared_geometry(
                va,
                PhysAddr::from_raw(target.frames[first].1.physical_address()),
                (last - first) * PAGE_SIZE,
                flags,
                self.geometry,
            );
            first = last;
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
        state.index = Some(insert(state.index.take(), node));
        state.len += 1;
        if pte_count != 0 {
            state.pending = true;
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
        if !ranges_cover(&ms.ranges, start.as_usize(), end) {
            return Err(MapError::NoRegion);
        }
        let mut next = Vec::new();
        next.try_reserve(ms.ranges.len().saturating_add(2))
            .map_err(|_| MapError::OutOfMemory)?;
        for range in &ms.ranges {
            let lo = range.start.as_usize();
            let hi = range.end();
            if hi < start.as_usize() || lo > end {
                next.push(*range);
                continue;
            }
            if !range.ceiling.contains(ceiling) {
                return Err(MapError::WidenDenied);
            }
            if lo < start.as_usize() {
                next.push(MapRange {
                    size: NonZeroUsize::new(start.as_usize() - lo).unwrap(),
                    ..*range
                });
            }
            let mid_lo = lo.max(start.as_usize());
            let mid_hi = hi.min(end);
            let current = (range.current - rwx) | ((range.current & rwx) & ceiling);
            if !valid_leaf_flags(current) {
                return Err(MapError::WidenDenied);
            }
            next.push(MapRange {
                start: VirtAddr::from_raw(mid_lo),
                size: NonZeroUsize::new(mid_hi - mid_lo + 1).unwrap(),
                ceiling,
                current,
                ..*range
            });
            if hi > end {
                let right = end + 1;
                next.push(MapRange {
                    start: VirtAddr::from_raw(right),
                    size: NonZeroUsize::new(hi - right + 1).unwrap(),
                    ..*range
                });
            }
        }
        let mut updates = Vec::new();
        let first = (start.as_usize() - map.initial_start.as_usize()) / PAGE_SIZE;
        let pages = size / PAGE_SIZE;
        updates
            .try_reserve(pages)
            .map_err(|_| MapError::OutOfMemory)?;
        for page in first..first + pages {
            let va = map.initial_start + page * PAGE_SIZE;
            let Some(range) = next.iter().find(|range| {
                range.start.as_usize() <= va.as_usize() && range.end() >= va.as_usize()
            }) else {
                return Err(MapError::NoRegion);
            };
            if ms.borrowed.is_some() || ms.frames.iter().any(|(p, _)| *p == page) {
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
        refresh(&mut state.index, map);
        state.pending = true;
        drop(state);
        self.finish_invalidation()
    }

    pub fn find(&self, address: VirtAddr) -> Option<Arc<Map>> {
        let state = self.ready_state().ok()?;
        find_map(state.index.as_deref(), address.as_usize())
    }

    #[cfg(debug_assertions)]
    pub(crate) fn leaf_span(&self, address: VirtAddr) -> Option<usize> {
        let state = self.ready_state().ok()?;
        find_map(state.index.as_deref(), address.as_usize())?;
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
            let Some(map) = find_map(state.index.as_deref(), at) else {
                return false;
            };
            let map_state = map.state.lock();
            let Some(range) = map_state
                .ranges
                .iter()
                .find(|range| range.start.as_usize() <= at && range.end() >= at)
            else {
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
            let Some(map) = find_map(state.index.as_deref(), at) else {
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
        find_map(state.index.as_deref(), address.as_usize())?;
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
        if !ranges_cover(&ms.ranges, start.as_usize(), end)
            || ms
                .ranges
                .iter()
                .filter(|r| r.start.as_usize() <= end && r.end() >= start.as_usize())
                .any(|r| !r.ceiling.contains(flags & rwx))
        {
            return Err(MapError::WidenDenied);
        }
        let changed = split_ranges(&ms.ranges, start.as_usize(), end, Some(flags))?;
        let first = (start.as_usize() - map.initial_start.as_usize()) / PAGE_SIZE;
        let pages = size / PAGE_SIZE;
        let mut updates = Vec::new();
        updates
            .try_reserve(pages)
            .map_err(|_| MapError::OutOfMemory)?;
        for page in first..first + pages {
            let va = map.initial_start + page * PAGE_SIZE;
            if ms.borrowed.is_some() || ms.frames.iter().any(|(p, _)| *p == page) {
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
        refresh(&mut state.index, map);
        state.pending = true;
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
        if !ranges_cover(&map_state.ranges, start.as_usize(), end) {
            return Err(MapError::NoRegion);
        }
        let ranges = split_ranges(&map_state.ranges, start.as_usize(), end, None)?;
        let next_node = if ranges.is_empty() {
            None
        } else {
            Some(
                Box::try_new(IndexNode::with_ranges(map.clone(), &ranges)?)
                    .map_err(|_| MapError::OutOfMemory)?,
            )
        };
        let first_page = (start.as_usize() - map.initial_start.as_usize()) / PAGE_SIZE;
        let last_page = (end - map.initial_start.as_usize()) / PAGE_SIZE;
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
        let (root, removed) = remove_map(state.index.take(), map);
        debug_assert!(removed, "live map must have an index node");
        state.index = match next_node {
            Some(node) => Some(insert(root, node)),
            None => {
                state.len -= 1;
                root
            }
        };
        state.pending = true;
        drop(state);
        self.finish_invalidation()
    }

    pub fn retry_invalidation(&self) -> Result<(), MapError> {
        if !self.state.lock().pending {
            return Ok(());
        }
        self.finish_invalidation()
    }

    fn finish_invalidation(&self) -> Result<(), MapError> {
        loop {
            let mut state = self.state.lock();
            if !state.pending {
                return Ok(());
            }
            if state.fencing {
                drop(state);
                while self.state.lock().fencing {
                    core::hint::spin_loop();
                }
                continue;
            }
            state.fencing = true;
            drop(state);

            let result = asid::shootdown(self.asid).map_err(|_| MapError::Shootdown);
            let retired_tables;
            let retired_frames;
            let pending_maps;
            {
                let mut state = self.state.lock();
                state.fencing = false;
                if let Err(error) = result {
                    state.failed = true;
                    return Err(error);
                }
                retired_tables = core::mem::take(&mut state.retired_tables);
                retired_frames = core::mem::take(&mut state.retired_frames);
                pending_maps = core::mem::take(&mut state.pending_maps);
                state.pending = false;
                state.failed = false;
                if state.close_pending {
                    state.closed = true;
                    state.close_pending = false;
                }
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
            if state.closed {
                return Ok(());
            }
            if !state.pending {
                break state;
            }
            drop(state);
            self.finish_invalidation()?;
        };
        let mut maps = Vec::new();
        maps.try_reserve(state.len)
            .map_err(|_| MapError::OutOfMemory)?;
        collect_maps(state.index.as_deref(), &mut maps);
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
        state.index = None;
        state.len = 0;
        state.pending = true;
        state.close_pending = true;
        drop(state);
        self.finish_invalidation()
    }

    #[cfg(debug_assertions)]
    pub(crate) fn audit(&self) {
        let state = self.state.lock();
        let (_, _, count) = audit_index(state.index.as_deref(), None, None);
        assert_eq!(count, state.len, "tree index length");
        audit_maps(state.index.as_deref(), self, &state.root, self.geometry);
        state.root.visit_leaves(
            (self.geometry.levels - 1) as usize,
            0,
            self.geometry,
            &mut |start, physical, flags, size| {
                let end = start
                    .as_usize()
                    .checked_add(size - 1)
                    .expect("leaf virtual interval is representable");
                let map = find_map(state.index.as_deref(), start.as_usize())
                    .expect("hardware leaf is absent from the Tree index");
                let end_map = find_map(state.index.as_deref(), end)
                    .expect("hardware leaf extends beyond the Tree index");
                assert!(
                    Arc::ptr_eq(&map, &end_map),
                    "hardware leaf crosses Map identity"
                );
                let map_state = map.state.lock();
                assert!(
                    ranges_cover(&map_state.ranges, start.as_usize(), end),
                    "hardware leaf crosses an unmapped Map range"
                );
                let mut expected_flags: Option<PteFlags> = None;
                for range in map_state.ranges.iter().filter(|range| {
                    range.start.as_usize() <= end && range.end() >= start.as_usize()
                }) {
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
                    let first_page = (start.as_usize() - map.initial_start.as_usize()) / PAGE_SIZE;
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
        self.state.lock().closed
    }
}

#[cfg(debug_assertions)]
fn audit_maps(node: Option<&IndexNode>, tree: &Tree, root: &TableNode, geo: mode::Geo) {
    let Some(node) = node else { return };
    audit_maps(node.left.as_deref(), tree, root, geo);
    let map = &node.map;
    assert!(
        core::ptr::eq(map.tree.as_ptr(), tree as *const Tree),
        "indexed Map belongs to another Tree"
    );
    let map_state = map.state.lock();
    assert!(
        map_state.borrowed.is_none() || map_state.frames.is_empty(),
        "Map is both borrowed and Frame-backed"
    );
    assert!(
        map_state.borrowed.is_none() || !map_state.lazy,
        "borrowed Map is marked lazy"
    );
    assert!(
        map_state
            .frames
            .windows(2)
            .all(|pair| pair[0].0 < pair[1].0),
        "owned Frame indexes are not strictly ordered"
    );
    for (page, frame) in &map_state.frames {
        let offset = page
            .checked_mul(PAGE_SIZE)
            .expect("Frame page offset overflow");
        assert!(
            offset < map.initial_size.get(),
            "owned Frame exceeds initial Map extent"
        );
        let address = map.initial_start + offset;
        assert!(
            ranges_cover(&map_state.ranges, address.as_usize(), address.as_usize()),
            "owned Frame lies outside live Map ranges"
        );
        let (physical, flags) = root
            .walk_ref_geometry(address, geo)
            .expect("owned Frame has no hardware PTE");
        assert_eq!(
            physical.as_usize(),
            frame.physical_address(),
            "owned Frame PTE physical address"
        );
        let expected = map_state
            .ranges
            .iter()
            .find(|range| {
                range.start.as_usize() <= address.as_usize() && range.end() >= address.as_usize()
            })
            .expect("Frame range exists")
            .current;
        assert_eq!(
            (flags
                & (PteFlags::V
                    | PteFlags::R
                    | PteFlags::W
                    | PteFlags::X
                    | PteFlags::U
                    | PteFlags::G))
                .bits(),
            (expected
                & (PteFlags::V
                    | PteFlags::R
                    | PteFlags::W
                    | PteFlags::X
                    | PteFlags::U
                    | PteFlags::G))
                .bits(),
            "Frame PTE permissions differ from Map state"
        );
    }
    if let Some(region) = &map_state.borrowed {
        assert!(
            map_state
                .borrowed_offset
                .checked_add(map.initial_size.get())
                .is_some_and(|end| end <= region.size.get()),
            "borrowed Map exceeds its reserved physical region"
        );
    }
    if !map_state.lazy || map_state.borrowed.is_some() {
        for range in &map_state.ranges {
            let mut offset = 0;
            while offset < range.size.get() {
                let address = range.start + offset;
                let span = root.leaf_span_geometry(address, geo);
                let step = match span {
                    Some(span) => {
                        assert!(
                            span <= range.size.get() - offset,
                            "hardware leaf crosses a live range boundary"
                        );
                        span
                    }
                    None if map_state.lazy => PAGE_SIZE,
                    None => panic!("eager live Map range has an unmapped page"),
                };
                offset += step;
            }
        }
    }
    drop(map_state);
    audit_maps(node.right.as_deref(), tree, root, geo);
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

fn valid_leaf_flags(flags: PteFlags) -> bool {
    let rwx = flags & (PteFlags::R | PteFlags::W | PteFlags::X);
    !rwx.is_empty() && (!rwx.contains(PteFlags::W) || rwx.contains(PteFlags::R))
}

fn ensure_map(tree: &Tree, map: &Arc<Map>, state: &TreeState) -> Result<(), MapError> {
    let owner = map.tree.upgrade().ok_or(MapError::NoRegion)?;
    if state.closed
        || !core::ptr::eq(Arc::as_ptr(&owner), tree)
        || map.state.lock().ranges.is_empty()
    {
        return Err(MapError::NoRegion);
    }
    Ok(())
}

fn ranges_cover(ranges: &[MapRange], lo: usize, hi: usize) -> bool {
    let mut at = lo;
    for range in ranges {
        if range.end() < at {
            continue;
        }
        if range.start.as_usize() > at {
            return false;
        }
        if range.end() >= hi {
            return true;
        }
        at = range.end() + 1;
    }
    false
}

fn split_ranges(
    ranges: &[MapRange],
    lo: usize,
    hi: usize,
    protect: Option<PteFlags>,
) -> Result<Vec<MapRange>, MapError> {
    let mut out = Vec::new();
    out.try_reserve(ranges.len().saturating_add(2))
        .map_err(|_| MapError::OutOfMemory)?;
    for range in ranges {
        let rlo = range.start.as_usize();
        let rhi = range.end();
        if rhi < lo || rlo > hi {
            out.push(*range);
            continue;
        }
        if rlo < lo {
            out.push(MapRange {
                start: range.start,
                size: NonZeroUsize::new(lo - rlo).unwrap(),
                ..*range
            });
        }
        if let Some(flags) = protect {
            let part_lo = rlo.max(lo);
            let part_hi = rhi.min(hi);
            out.push(MapRange {
                start: VirtAddr::from_raw(part_lo),
                size: NonZeroUsize::new(part_hi - part_lo + 1).unwrap(),
                current: (range.current - (PteFlags::R | PteFlags::W | PteFlags::X))
                    | (flags & (PteFlags::R | PteFlags::W | PteFlags::X)),
                ..*range
            });
        }
        if rhi > hi {
            let part_lo = hi + 1;
            out.push(MapRange {
                start: VirtAddr::from_raw(part_lo),
                size: NonZeroUsize::new(rhi - part_lo + 1).unwrap(),
                ..*range
            });
        }
    }
    Ok(out)
}

#[cfg(debug_assertions)]
fn audit_index(
    node: Option<&IndexNode>,
    lower: Option<usize>,
    upper: Option<usize>,
) -> (usize, usize, usize) {
    let Some(node) = node else { return (0, 0, 0) };
    let key = node.start.as_usize();
    assert!(lower.is_none_or(|lo| lo < key), "AVL key below lower bound");
    assert!(upper.is_none_or(|hi| key < hi), "AVL key above upper bound");
    let (left_height, left_end, left_count) = audit_index(node.left.as_deref(), lower, Some(key));
    let (right_height, right_end, right_count) =
        audit_index(node.right.as_deref(), Some(key), upper);
    let map_state = node.map.state.lock();
    assert!(
        !map_state.ranges.is_empty(),
        "indexed map has no active ranges"
    );
    assert!(
        map_state
            .ranges
            .windows(2)
            .all(|pair| pair[0].end() < pair[1].start.as_usize()),
        "map ranges overlap or are unordered"
    );
    for range in &map_state.ranges {
        assert!(
            !overlaps_other(node, &node.map, range.start.as_usize(), range.end()),
            "distinct Maps overlap in the Tree index"
        );
    }
    assert_eq!(
        key,
        map_state
            .ranges
            .iter()
            .map(|r| r.start.as_usize())
            .min()
            .unwrap(),
        "AVL key differs from active Map range"
    );
    let local_end = map_state.ranges.iter().map(|r| r.end()).max().unwrap();
    let expected_height = 1 + left_height.max(right_height);
    assert_eq!(node.height as usize, expected_height, "AVL height cache");
    assert!(left_height.abs_diff(right_height) <= 1, "AVL balance");
    let expected_end = local_end.max(left_end).max(right_end);
    assert_eq!(node.max_end, expected_end, "AVL max_end cache");
    (expected_height, expected_end, left_count + right_count + 1)
}

#[cfg(debug_assertions)]
fn overlaps_other(node: &IndexNode, map: &Arc<Map>, lo: usize, hi: usize) -> bool {
    if node.max_end < lo {
        return false;
    }
    if node
        .left
        .as_ref()
        .is_some_and(|left| left.max_end >= lo && overlaps_other(left, map, lo, hi))
    {
        return true;
    }
    if !Arc::ptr_eq(&node.map, map) && node.map.overlaps(lo, hi) {
        return true;
    }
    node.start.as_usize() <= hi
        && node
            .right
            .as_ref()
            .is_some_and(|right| overlaps_other(right, map, lo, hi))
}

fn collect_maps(node: Option<&IndexNode>, maps: &mut Vec<Arc<Map>>) {
    let Some(node) = node else { return };
    collect_maps(node.left.as_deref(), maps);
    maps.push(node.map.clone());
    collect_maps(node.right.as_deref(), maps);
}

fn find_map(root: Option<&IndexNode>, address: usize) -> Option<Arc<Map>> {
    let node = root?;
    if node
        .left
        .as_ref()
        .is_some_and(|left| left.max_end >= address)
    {
        if let Some(map) = find_map(node.left.as_deref(), address) {
            return Some(map);
        }
    }
    if node.map.overlaps(address, address) {
        return Some(node.map.clone());
    }
    (node.start.as_usize() <= address)
        .then(|| find_map(node.right.as_deref(), address))
        .flatten()
}

fn first_overlap(root: Option<&IndexNode>, lo: usize, hi: usize) -> Option<Arc<Map>> {
    let node = root?;
    if node.max_end < lo {
        return None;
    }
    if node.left.as_ref().is_some_and(|left| left.max_end >= lo) {
        if let Some(map) = first_overlap(node.left.as_deref(), lo, hi) {
            return Some(map);
        }
    }
    if node.map.overlaps(lo, hi) {
        return Some(node.map.clone());
    }
    if node.start.as_usize() <= hi {
        first_overlap(node.right.as_deref(), lo, hi)
    } else {
        None
    }
}

fn height(node: &Option<Box<IndexNode>>) -> u8 {
    node.as_ref().map_or(0, |node| node.height)
}

fn update(node: &mut IndexNode) {
    node.height = 1 + height(&node.left).max(height(&node.right));
    {
        let state = node.map.state.lock();
        if let Some(start) = state.ranges.iter().map(|range| range.start).min() {
            node.start = start;
        }
        node.max_end = state
            .ranges
            .iter()
            .map(|range| range.end())
            .max()
            .unwrap_or(0);
    }
    if let Some(left) = node.left.as_ref() {
        node.max_end = node.max_end.max(left.max_end);
    }
    if let Some(right) = node.right.as_ref() {
        node.max_end = node.max_end.max(right.max_end);
    }
}

fn rotate_left(mut node: Box<IndexNode>) -> Box<IndexNode> {
    let mut right = node.right.take().expect("AVL right child");
    node.right = right.left.take();
    update(&mut node);
    right.left = Some(node);
    update(&mut right);
    right
}

fn rotate_right(mut node: Box<IndexNode>) -> Box<IndexNode> {
    let mut left = node.left.take().expect("AVL left child");
    node.left = left.right.take();
    update(&mut node);
    left.right = Some(node);
    update(&mut left);
    left
}

fn balance(mut node: Box<IndexNode>) -> Box<IndexNode> {
    update(&mut node);
    if height(&node.left) > height(&node.right) + 1 {
        if height(&node.left.as_ref().unwrap().right) > height(&node.left.as_ref().unwrap().left) {
            node.left = Some(rotate_left(node.left.take().unwrap()));
        }
        rotate_right(node)
    } else if height(&node.right) > height(&node.left) + 1 {
        if height(&node.right.as_ref().unwrap().left) > height(&node.right.as_ref().unwrap().right)
        {
            node.right = Some(rotate_right(node.right.take().unwrap()));
        }
        rotate_left(node)
    } else {
        node
    }
}

fn insert(root: Option<Box<IndexNode>>, node: Box<IndexNode>) -> Box<IndexNode> {
    let Some(mut root) = root else { return node };
    if node.start < root.start {
        root.left = Some(insert(root.left.take(), node));
    } else {
        root.right = Some(insert(root.right.take(), node));
    }
    balance(root)
}

fn take_min(mut node: Box<IndexNode>) -> (Option<Box<IndexNode>>, Box<IndexNode>) {
    let Some(left) = node.left.take() else {
        let right = node.right.take();
        return (right, node);
    };
    let (left, min) = take_min(left);
    node.left = left;
    (Some(balance(node)), min)
}

fn detach_node(mut node: Box<IndexNode>) -> Option<Box<IndexNode>> {
    let left = node.left.take();
    let right = node.right.take();
    match (left, right) {
        (None, right) => right,
        (left, None) => left,
        (left, Some(right)) => {
            let (right, mut next) = take_min(right);
            next.left = left;
            next.right = right;
            Some(balance(next))
        }
    }
}

fn remove_map(root: Option<Box<IndexNode>>, map: &Arc<Map>) -> (Option<Box<IndexNode>>, bool) {
    let Some(mut node) = root else {
        return (None, false);
    };
    if Arc::ptr_eq(&node.map, map) {
        return (detach_node(node), true);
    }
    let (left, found) = remove_map(node.left.take(), map);
    node.left = left;
    if found {
        return (Some(balance(node)), true);
    }
    let (right, found) = remove_map(node.right.take(), map);
    node.right = right;
    if found {
        (Some(balance(node)), true)
    } else {
        (Some(node), false)
    }
}

fn refresh(root: &mut Option<Box<IndexNode>>, map: &Arc<Map>) -> bool {
    let Some(node) = root.as_deref_mut() else {
        return false;
    };
    if Arc::ptr_eq(&node.map, map) {
        update(node);
        return true;
    }
    if refresh(&mut node.left, map) || refresh(&mut node.right, map) {
        update(node);
        true
    } else {
        false
    }
}
