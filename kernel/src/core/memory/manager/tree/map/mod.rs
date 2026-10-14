use alloc::sync::{Arc, Weak};
use alloc::vec::Vec;
use core::num::NonZeroUsize;

use crate::core::memory::PAGE_SIZE;
use crate::core::memory::manager::addr::VirtAddr;
use crate::core::memory::manager::entry::PteFlags;
#[cfg(debug_assertions)]
use crate::core::memory::manager::frame::Frame;
use crate::core::memory::manager::MapError;
use crate::machine::sync::{Level, SpinLock};

use super::Tree;

pub(super) mod range;
mod run;
mod runs;
mod state;

pub use range::MapRange;

use state::MapState;

/// 一段虚拟区间映射。
pub(crate) struct Map {
    pub(super) tree: Weak<Tree>,
    pub(super) initial_start: VirtAddr,
    pub(super) initial_size: NonZeroUsize,
    pub(super) state: SpinLock<MapState>,
}

impl core::fmt::Debug for Map {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Map")
            .field("initial_start", &self.initial_start)
            .field("initial_size", &self.initial_size)
            .finish_non_exhaustive()
    }
}

impl Map {
    pub(super) fn new(
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
            state: SpinLock::new_level(Level::Map, MapState::new(ranges)),
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
        if !state.covers(start.as_usize(), end) || state.borrowed.is_some() {
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

    /// `address` 相对初始范围的页号；调用方保证它在初始范围内。
    pub(super) fn page_index(&self, address: VirtAddr) -> usize {
        (address.as_usize() - self.initial_start.as_usize()) / PAGE_SIZE
    }

    /// 活区间是否与 [lo, hi] 相交。
    pub(super) fn overlaps(&self, lo: usize, hi: usize) -> bool {
        self.state
            .lock()
            .ranges
            .iter()
            .any(|range| range.overlaps(lo, hi))
    }
}
