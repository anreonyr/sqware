use alloc::boxed::Box;
use alloc::sync::{Arc, Weak};
use alloc::vec::Vec;

use super::{RegionId, SpaceKind};
use crate::layout::{TEAM_FRAME_BASE, TEAM_FRAME_WINDOW_SIZE};
use crate::lock::{Level, SpinLock};
use crate::memory::PAGE_SIZE;
use crate::memory::manager::addr::{PhysAddr, VirtAddr};
use crate::memory::manager::entry::PteFlags;
use crate::memory::manager::{BorrowedRegion, Frame, Map, MapError, Tree};
use crate::work::unit::life::Life;

#[cfg(debug_assertions)]
static FAIL_CLEANUP: core::sync::atomic::AtomicBool = core::sync::atomic::AtomicBool::new(false);

#[cfg(debug_assertions)]
pub(crate) fn fail_cleanup_prepare() {
    assert!(crate::testing());
    FAIL_CLEANUP.store(true, core::sync::atomic::Ordering::Relaxed);
}

pub struct Space {
    pub(crate) tree: Arc<Tree>,
    allocator: SpinLock<super::allocator::VaAllocator>,
    pending_cleanup: SpinLock<Option<Box<CleanupNode>>>,
    kind: SpaceKind,
    life: Arc<Life>,
}

pub(super) struct CleanupNode {
    record: Option<Reclaim>,
    next: Option<Box<CleanupNode>>,
}

impl CleanupNode {
    pub(super) const fn empty() -> Self {
        Self {
            record: None,
            next: None,
        }
    }
}

pub(super) enum Reclaim {
    Mapping(Mapping),
    Range(RangeReclaim),
}

pub(super) struct RangeReclaim {
    maps: Vec<(Arc<Map>, VirtAddr, usize)>,
    tickets: Vec<usize>,
    cursor: usize,
}

pub struct SpaceBuilder {
    kind: SpaceKind,
    kernel: bool,
}

pub struct Reservation {
    tree: Weak<Tree>,
    region: RegionId,
    start: VirtAddr,
    size: usize,
    id: usize,
}

impl core::fmt::Debug for Reservation {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Reservation")
            .field("region", &self.region)
            .field("start", &self.start)
            .field("size", &self.size)
            .finish()
    }
}

pub struct Mapping {
    pub map: Arc<Map>,
    pub reservation: Reservation,
}

unsafe impl Send for Space {}
unsafe impl Sync for Space {}
// SAFETY: Tree and Map state is internally synchronized; allocator identity is immutable.
unsafe impl Send for Reservation {}
unsafe impl Sync for Reservation {}
unsafe impl Send for Mapping {}
unsafe impl Sync for Mapping {}

impl SpaceBuilder {
    pub fn kernel() -> Self {
        Self {
            kind: SpaceKind::Supervisor,
            kernel: true,
        }
    }

    pub fn supervisor() -> Self {
        Self {
            kind: SpaceKind::Supervisor,
            kernel: false,
        }
    }

    pub fn user() -> Self {
        Self {
            kind: SpaceKind::User,
            kernel: false,
        }
    }

    pub fn build(self) -> Result<Space, MapError> {
        let tree = if self.kernel {
            Tree::new_kernel()?
        } else {
            Tree::new()?
        };
        let mut allocator = super::allocator::VaAllocator::new();
        allocator.attach(
            RegionId::Normal,
            crate::layout::IMAGE_BASE.as_usize(),
            crate::memory::manager::mode::upper().as_usize(),
        )?;
        allocator.attach(
            RegionId::Kernel,
            TEAM_FRAME_BASE.as_usize(),
            TEAM_FRAME_BASE.as_usize() + TEAM_FRAME_WINDOW_SIZE,
        )?;
        if self.kernel {
            let dram = crate::platform::machine::info().dram;
            let base = crate::memory::manager::mode::lower()
                .as_usize()
                .checked_add(dram.base)
                .and_then(|base| base.checked_add(dram.size))
                .ok_or(MapError::DramOverlap)?;
            allocator.attach(
                RegionId::Resource,
                base,
                crate::layout::TRAP_STACK_BASE.as_usize(),
            )?;
        }
        let space = Space {
            tree,
            allocator: SpinLock::new_level(Level::Space, allocator),
            pending_cleanup: SpinLock::new_level(Level::Space, None),
            kind: self.kind,
            life: Life::try_new()?,
        };
        if !self.kernel {
            space.seed_trampoline()?;
        }
        Ok(space)
    }
}

impl Space {
    fn seed_trampoline(&self) -> Result<(), MapError> {
        let source = crate::work::unit::team::kernel()
            .expect("kernel team not initialized")
            .space
            .tree
            .clone();
        let (physical, flags) = source
            .translate(crate::layout::TRAMPOLINE)
            .ok_or(MapError::NoRegion)?;
        let region = unsafe { BorrowedRegion::new(physical, PAGE_SIZE)? };
        self.tree.map_borrowed(
            crate::layout::TRAMPOLINE,
            region,
            0,
            PAGE_SIZE,
            flags,
            flags,
        )?;
        Ok(())
    }
}

impl Space {
    pub fn kind(&self) -> SpaceKind {
        self.kind
    }

    pub fn pte_policy(&self, flags: PteFlags) -> PteFlags {
        if self.kind.is_supervisor() {
            flags - PteFlags::U
        } else {
            flags | PteFlags::U
        }
    }

    fn mapping_policy(&self, region: RegionId, flags: PteFlags) -> PteFlags {
        if region != RegionId::Normal {
            flags - PteFlags::U
        } else {
            self.pte_policy(flags)
        }
    }

    pub fn life(&self) -> Weak<Life> {
        Arc::downgrade(&self.life)
    }

    pub(crate) fn attach_region(
        &self,
        region: RegionId,
        base: usize,
        edge: usize,
    ) -> Result<(), MapError> {
        self.allocator.lock().configure(region, base, edge)
    }

    pub(crate) fn advance_region(&self, region: RegionId, base: usize) -> Result<(), MapError> {
        self.allocator.lock().advance(region, base)
    }

    pub fn mapping_at(&self, start: VirtAddr, size: usize) -> Result<Mapping, MapError> {
        if size == 0 || start.offset() != 0 || !size.is_multiple_of(PAGE_SIZE) {
            return Err(MapError::NotAligned);
        }
        let map = self.tree.find(start).ok_or(MapError::NoRegion)?;
        let (map_start, map_size) = map.initial_range();
        if map_start != start || map_size != size || !map.is_live() {
            return Err(MapError::NoRegion);
        }
        let region = self
            .allocator
            .lock()
            .exact_reservation(start.as_usize(), size)
            .ok_or(MapError::NoRegion)?;
        Ok(Mapping {
            map,
            reservation: Reservation {
                tree: Arc::downgrade(&self.tree),
                region: region.0,
                start,
                size,
                id: region.1,
            },
        })
    }

    pub fn reserve(
        &self,
        region: RegionId,
        start: VirtAddr,
        size: usize,
    ) -> Result<Reservation, MapError> {
        if size == 0 || start.offset() != 0 || !size.is_multiple_of(PAGE_SIZE) {
            return Err(MapError::NotAligned);
        }
        let id = self
            .allocator
            .lock()
            .reserve(region, start.as_usize(), size)?;
        Ok(Reservation {
            tree: Arc::downgrade(&self.tree),
            region,
            start,
            size,
            id,
        })
    }

    pub fn allocate_low(&self, region: RegionId, size: usize) -> Result<Reservation, MapError> {
        let (start, id) = self.allocator.lock().allocate_low(region, size)?;
        Ok(Reservation {
            tree: Arc::downgrade(&self.tree),
            region,
            start: VirtAddr::from_raw(start),
            size,
            id,
        })
    }

    pub fn allocate_high(&self, region: RegionId, size: usize) -> Result<Reservation, MapError> {
        let (start, id) = self.allocator.lock().allocate_high(region, size)?;
        Ok(Reservation {
            tree: Arc::downgrade(&self.tree),
            region,
            start: VirtAddr::from_raw(start),
            size,
            id,
        })
    }

    pub fn split_reservation(
        &self,
        reservation: Reservation,
        left_size: usize,
    ) -> Result<(Reservation, Reservation), MapError> {
        self.check_reservation(&reservation)?;
        let split = self.allocator.lock().split(
            reservation.region,
            reservation.start.as_usize(),
            reservation.size,
            reservation.id,
            left_size,
        );
        let (right_start, left_id, right_id) = match split {
            Ok(split) => split,
            Err(error) => {
                let _ = self.allocator.lock().release(
                    reservation.region,
                    reservation.start.as_usize(),
                    reservation.size,
                    reservation.id,
                );
                return Err(error);
            }
        };
        let owner = Arc::downgrade(&self.tree);
        Ok((
            Reservation {
                tree: owner.clone(),
                region: reservation.region,
                start: reservation.start,
                size: left_size,
                id: left_id,
            },
            Reservation {
                tree: owner,
                region: reservation.region,
                start: VirtAddr::from_raw(right_start),
                size: reservation.size - left_size,
                id: right_id,
            },
        ))
    }

    pub fn release_reservation(&self, reservation: Reservation) -> Result<(), MapError> {
        self.check_reservation(&reservation)?;
        if self.tree.overlaps(reservation.start, reservation.size)? {
            return Err(MapError::AlreadyMapped);
        }
        if self.allocator.lock().release(
            reservation.region,
            reservation.start.as_usize(),
            reservation.size,
            reservation.id,
        ) {
            Ok(())
        } else {
            Err(MapError::NoRegion)
        }
    }

    fn enqueue_cleanup(&self, mut node: Box<CleanupNode>) {
        let mut pending = self.pending_cleanup.lock();
        node.next = pending.take();
        *pending = Some(node);
    }

    fn take_cleanup(&self) -> Option<Box<CleanupNode>> {
        let mut pending = self.pending_cleanup.lock();
        let mut node = pending.take()?;
        *pending = node.next.take();
        Some(node)
    }

    fn prepare_mapping_cleanup(&self, reservation: &Reservation) -> Result<(), MapError> {
        #[cfg(debug_assertions)]
        if FAIL_CLEANUP.swap(false, core::sync::atomic::Ordering::Relaxed) {
            return Err(MapError::OutOfMemory);
        }
        self.allocator.lock().prepare_cleanup(reservation.id)
    }

    fn cancel_mapping_cleanup(&self, reservation: &Reservation) {
        let _ = self.allocator.lock().take_cleanup(reservation.id);
    }

    fn record_map_failure(
        &self,
        reservation: Reservation,
        error: MapError,
    ) -> Result<Mapping, MapError> {
        let map = self.tree.probe(reservation.start).filter(|map| {
            let (start, size) = map.initial_range();
            start == reservation.start
                && size == reservation.size
                && map
                    .tree()
                    .is_some_and(|tree| Arc::ptr_eq(&tree, &self.tree))
        });
        if let Some(map) = map {
            let node = self.allocator.lock().take_cleanup(reservation.id);
            if let Some(mut node) = node {
                node.record = Some(Reclaim::Mapping(Mapping { map, reservation }));
                self.enqueue_cleanup(node);
            }
        } else {
            self.cancel_mapping_cleanup(&reservation);
            let _ = self.allocator.lock().release(
                reservation.region,
                reservation.start.as_usize(),
                reservation.size,
                reservation.id,
            );
        }
        Err(error)
    }

    fn ensure_unmapped_reservation(&self, reservation: &Reservation) -> Result<(), MapError> {
        if self.tree.overlaps(reservation.start, reservation.size)? {
            let _ = self.allocator.lock().release(
                reservation.region,
                reservation.start.as_usize(),
                reservation.size,
                reservation.id,
            );
            return Err(MapError::AlreadyMapped);
        }
        Ok(())
    }

    pub fn map_frames(
        &self,
        reservation: Reservation,
        frames: Vec<Arc<Frame>>,
        ceiling: PteFlags,
        current: PteFlags,
    ) -> Result<Mapping, MapError> {
        self.check_reservation(&reservation)?;
        if frames.len().checked_mul(PAGE_SIZE) != Some(reservation.size) {
            let _ = self.release_reservation(reservation);
            return Err(MapError::NotAligned);
        }
        self.ensure_unmapped_reservation(&reservation)?;
        if let Err(error) = self.prepare_mapping_cleanup(&reservation) {
            let _ = self.allocator.lock().release(
                reservation.region,
                reservation.start.as_usize(),
                reservation.size,
                reservation.id,
            );
            return Err(error);
        }
        match self.tree.map_frames(
            reservation.start,
            frames,
            ceiling,
            self.mapping_policy(reservation.region, current),
        ) {
            Ok(map) => Ok(Mapping { map, reservation }),
            Err(error) => self.record_map_failure(reservation, error),
        }
    }

    pub fn map_lazy(
        &self,
        reservation: Reservation,
        ceiling: PteFlags,
        current: PteFlags,
    ) -> Result<Mapping, MapError> {
        self.check_reservation(&reservation)?;
        self.ensure_unmapped_reservation(&reservation)?;
        if let Err(error) = self.prepare_mapping_cleanup(&reservation) {
            let _ = self.allocator.lock().release(
                reservation.region,
                reservation.start.as_usize(),
                reservation.size,
                reservation.id,
            );
            return Err(error);
        }
        match self.tree.map_lazy(
            reservation.start,
            reservation.size,
            ceiling,
            self.mapping_policy(reservation.region, current),
        ) {
            Ok(map) => Ok(Mapping { map, reservation }),
            Err(error) => self.record_map_failure(reservation, error),
        }
    }

    /// The caller keeps external physical storage reserved for the full mapping lifetime.
    pub unsafe fn map_borrowed(
        &self,
        reservation: Reservation,
        physical: PhysAddr,
        ceiling: PteFlags,
        current: PteFlags,
    ) -> Result<Mapping, MapError> {
        self.check_reservation(&reservation)?;
        self.ensure_unmapped_reservation(&reservation)?;
        let region = match unsafe { BorrowedRegion::new(physical, reservation.size) } {
            Ok(region) => region,
            Err(error) => {
                let _ = self.release_reservation(reservation);
                return Err(error);
            }
        };
        if let Err(error) = self.prepare_mapping_cleanup(&reservation) {
            let _ = self.allocator.lock().release(
                reservation.region,
                reservation.start.as_usize(),
                reservation.size,
                reservation.id,
            );
            return Err(error);
        }
        match self.tree.map_borrowed(
            reservation.start,
            region,
            0,
            reservation.size,
            ceiling,
            self.mapping_policy(reservation.region, current),
        ) {
            Ok(map) => Ok(Mapping { map, reservation }),
            Err(error) => self.record_map_failure(reservation, error),
        }
    }

    pub fn derive(
        &self,
        reservation: Reservation,
        source: &Arc<Map>,
        source_offset: usize,
        ceiling: PteFlags,
        current: PteFlags,
    ) -> Result<Mapping, MapError> {
        self.check_reservation(&reservation)?;
        self.ensure_unmapped_reservation(&reservation)?;
        if let Err(error) = self.prepare_mapping_cleanup(&reservation) {
            let _ = self.allocator.lock().release(
                reservation.region,
                reservation.start.as_usize(),
                reservation.size,
                reservation.id,
            );
            return Err(error);
        }
        match self.tree.derive_map(
            source,
            source_offset,
            reservation.size,
            reservation.start,
            ceiling,
            self.mapping_policy(reservation.region, current),
        ) {
            Ok(map) => Ok(Mapping { map, reservation }),
            Err(error) => self.record_map_failure(reservation, error),
        }
    }

    pub fn user_range(addr: usize, size: usize) -> bool {
        let upper = crate::memory::manager::mode::upper().as_usize();
        addr < upper && addr.checked_add(size).is_some_and(|end| end <= upper)
    }
    pub fn retry_invalidation(&self) -> Result<(), MapError> {
        self.tree.retry_invalidation()?;
        while let Some(mut node) = self.take_cleanup() {
            let result = match node.record.as_mut().expect("cleanup record") {
                Reclaim::Mapping(mapping) => self.unmap(mapping),
                Reclaim::Range(range) => self.finish_range(range),
            };
            if let Err(error) = result {
                self.enqueue_cleanup(node);
                return Err(error);
            }
        }
        Ok(())
    }

    pub(crate) fn unmap_range(&self, start: VirtAddr, size: usize) -> Result<(), MapError> {
        if !Self::user_range(start.as_usize(), size)
            || start.offset() != 0
            || !size.is_multiple_of(PAGE_SIZE)
        {
            return Err(MapError::NotAligned);
        }
        self.retry_invalidation()?;
        let end = start.as_usize() + size;
        let ranges = self.tree.ranges()?;
        let mut maps = Vec::new();
        maps.try_reserve(ranges.len())
            .map_err(|_| MapError::OutOfMemory)?;
        for (map, range) in ranges {
            let lo = start.as_usize().max(range.start.as_usize());
            if range.start.as_usize() >= end || range.end() < start.as_usize() {
                continue;
            }
            let hi = (range.end() + 1).min(end);
            if lo < hi {
                maps.push((map, VirtAddr::wrap(lo), hi - lo));
            }
        }
        if maps.is_empty() {
            return Err(MapError::NoRegion);
        }
        let mut cuts = Vec::new();
        for (_, at, len) in &maps {
            let part = self
                .allocator
                .lock()
                .cuts(RegionId::Normal, at.as_usize(), *len)?;
            if part.iter().map(|(_, len, _)| len).sum::<usize>() != *len {
                return Err(MapError::NoRegion);
            }
            cuts.try_reserve(part.len())
                .map_err(|_| MapError::OutOfMemory)?;
            cuts.extend(part);
        }
        let mut tickets = Vec::new();
        tickets
            .try_reserve(cuts.len())
            .map_err(|_| MapError::OutOfMemory)?;
        let mut cleanup = Box::try_new(CleanupNode::empty()).map_err(|_| MapError::OutOfMemory)?;
        {
            let mut allocator = self.allocator.lock();
            if let Err(error) = allocator.reserve_cuts(RegionId::Normal, cuts.len() * 2) {
                return Err(error);
            }
            for (at, len, id) in cuts {
                tickets.push(allocator.retire(RegionId::Normal, at, len, id));
            }
        }
        let mut record = RangeReclaim {
            maps,
            tickets,
            cursor: 0,
        };
        let result = self.finish_range(&mut record);
        if result.is_err() {
            cleanup.record = Some(Reclaim::Range(record));
            self.enqueue_cleanup(cleanup);
        }
        result
    }

    fn finish_range(&self, record: &mut RangeReclaim) -> Result<(), MapError> {
        if self.tree.has_pending_invalidation() {
            self.tree.retry_invalidation()?;
        }
        while record.cursor < record.maps.len() {
            let (map, start, size) = &record.maps[record.cursor];
            let end = start.as_usize() + size;
            let ranges = map.ranges()?;
            for range in ranges {
                let lo = start.as_usize().max(range.start.as_usize());
                let hi = end.min(range.end() + 1);
                if lo < hi {
                    self.tree.unmap(map, VirtAddr::wrap(lo), hi - lo)?;
                }
            }
            record.cursor += 1;
        }
        let mut allocator = self.allocator.lock();
        for ticket in record.tickets.drain(..) {
            allocator.reclaim(RegionId::Normal, ticket);
        }
        Ok(())
    }

    pub(crate) fn dispose(&self, mapping: Mapping) -> Result<(), MapError> {
        self.check_tree_identity(&mapping.reservation)?;
        let id = mapping.reservation.id;
        let mut node = self.allocator.lock().take_cleanup(id);
        if node.is_none() {
            if mapping.map.ranges()?.is_empty() {
                return Ok(());
            }
            return Err(MapError::Shootdown);
        }
        let mut node = node.take().expect("cleanup node");
        node.record = Some(Reclaim::Mapping(mapping));
        let result = match node.record.as_ref().expect("mapping cleanup") {
            Reclaim::Mapping(mapping) => self.unmap(mapping),
            Reclaim::Range(_) => unreachable!(),
        };
        if let Err(error) = result {
            self.enqueue_cleanup(node);
            return Err(error);
        }
        Ok(())
    }

    pub fn unmap(&self, mapping: &Mapping) -> Result<(), MapError> {
        self.check_tree_identity(&mapping.reservation)?;
        let (start, size) = mapping.map.initial_range();
        if start != mapping.reservation.start || size != mapping.reservation.size {
            return Err(MapError::NoRegion);
        }
        if self.tree.has_pending_invalidation() {
            self.tree.retry_invalidation()?;
        }
        let ranges = mapping.map.ranges()?;
        if ranges.is_empty() {
            self.release_range(&mapping.reservation);
            return Ok(());
        }
        self.check_reservation(&mapping.reservation)?;
        loop {
            let ranges = mapping.map.ranges()?;
            let Some(range) = ranges.first().copied() else {
                self.release_range(&mapping.reservation);
                return Ok(());
            };
            self.tree
                .unmap(&mapping.map, range.start, range.size.get())?;
        }
    }

    fn release_range(&self, reservation: &Reservation) {
        let ticket = self.allocator.lock().retire(
            reservation.region,
            reservation.start.as_usize(),
            reservation.size,
            reservation.id,
        );
        self.allocator.lock().reclaim(reservation.region, ticket);
    }

    fn check_tree_identity(&self, reservation: &Reservation) -> Result<(), MapError> {
        let owner = reservation.tree.upgrade().ok_or(MapError::NoRegion)?;
        if !Arc::ptr_eq(&owner, &self.tree) {
            return Err(MapError::NoRegion);
        }
        Ok(())
    }

    fn check_reservation(&self, reservation: &Reservation) -> Result<(), MapError> {
        let owner = reservation.tree.upgrade().ok_or(MapError::NoRegion)?;
        if !Arc::ptr_eq(&owner, &self.tree)
            || !self
                .allocator
                .lock()
                .has_id(reservation.region, reservation.id)
        {
            return Err(MapError::NoRegion);
        }
        Ok(())
    }
}

impl Reservation {
    pub fn start(&self) -> VirtAddr {
        self.start
    }
    pub fn size(&self) -> usize {
        self.size
    }
}
