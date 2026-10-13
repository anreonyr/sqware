use alloc::boxed::Box;
use alloc::vec::Vec;
use core::sync::atomic::{AtomicUsize, Ordering};

use crate::memory::manager::MapError;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum RegionId {
    Normal,
    Kernel,
    Resource,
}

struct Slot {
    start: usize,
    size: usize,
    ticket: usize,
    id: usize,
}

struct Region {
    id: RegionId,
    base: usize,
    edge: usize,
    reserved: Vec<Slot>,
}

pub(crate) struct VaAllocator {
    regions: Vec<Region>,
    cleanups: Vec<(usize, Box<super::space::CleanupNode>)>,
}

impl VaAllocator {
    pub(crate) const fn new() -> Self {
        Self {
            regions: Vec::new(),
            cleanups: Vec::new(),
        }
    }

    pub(crate) fn attach(
        &mut self,
        id: RegionId,
        base: usize,
        edge: usize,
    ) -> Result<(), MapError> {
        if base >= edge || self.regions.iter().any(|r| r.id == id) {
            return Err(MapError::NoRegion);
        }
        self.regions
            .try_reserve(1)
            .map_err(|_| MapError::OutOfMemory)?;
        self.regions.push(Region {
            id,
            base,
            edge,
            reserved: Vec::new(),
        });
        Ok(())
    }

    pub(crate) fn configure(
        &mut self,
        id: RegionId,
        base: usize,
        edge: usize,
    ) -> Result<(), MapError> {
        if base >= edge {
            return Err(MapError::NoRegion);
        }
        if let Some(region) = self.regions.iter_mut().find(|region| region.id == id) {
            if region.reserved.iter().any(|slot| {
                slot.start < base
                    || slot
                        .start
                        .checked_add(slot.size)
                        .is_none_or(|end| end > edge)
            }) {
                return Err(MapError::AlreadyMapped);
            }
            region.base = base;
            region.edge = edge;
            return Ok(());
        }
        self.attach(id, base, edge)
    }

    pub(crate) fn advance(&mut self, id: RegionId, base: usize) -> Result<(), MapError> {
        let region = self.region_mut(id).ok_or(MapError::NoRegion)?;
        if base < region.base || base >= region.edge {
            return Err(MapError::NoRegion);
        }
        region.base = base;
        Ok(())
    }

    pub(crate) fn allocate_low(
        &mut self,
        id: RegionId,
        size: usize,
    ) -> Result<(usize, usize), MapError> {
        Self::check_size(size)?;
        let start = self
            .gaps(id)
            .find_map(|(lo, hi)| (hi - lo >= size).then_some(lo))
            .ok_or(MapError::OutOfMemory)?;
        let id = self.reserve(id, start, size)?;
        Ok((start, id))
    }

    pub(crate) fn allocate_high(
        &mut self,
        id: RegionId,
        size: usize,
    ) -> Result<(usize, usize), MapError> {
        Self::check_size(size)?;
        let start = self
            .gaps(id)
            .rev()
            .find_map(|(lo, hi)| hi.checked_sub(size).filter(|&start| start >= lo))
            .ok_or(MapError::OutOfMemory)?;
        let id = self.reserve(id, start, size)?;
        Ok((start, id))
    }

    fn check_size(size: usize) -> Result<(), MapError> {
        if size == 0 || !size.is_multiple_of(crate::memory::PAGE_SIZE) {
            Err(MapError::NotAligned)
        } else {
            Ok(())
        }
    }

    pub(crate) fn gaps(
        &self,
        id: RegionId,
    ) -> impl DoubleEndedIterator<Item = (usize, usize)> + '_ {
        self.region(id).into_iter().flat_map(|r| {
            let first = r.reserved.partition_point(|s| s.start + s.size <= r.base);
            (first..=r.reserved.len())
                .map(move |i| {
                    let start = if i == first {
                        r.base.max(
                            r.reserved
                                .get(i.wrapping_sub(1))
                                .map_or(r.base, |s| s.start + s.size),
                        )
                    } else {
                        r.reserved[i - 1].start + r.reserved[i - 1].size
                    };
                    let end = r.reserved.get(i).map_or(r.edge, |s| s.start);
                    (start, end)
                })
                .filter(|&(start, end)| start < end)
        })
    }

    pub(crate) fn reserve(
        &mut self,
        id: RegionId,
        addr: usize,
        size: usize,
    ) -> Result<usize, MapError> {
        let end = addr.checked_add(size).ok_or(MapError::NoRegion)?;
        let r = self.region_mut(id).ok_or(MapError::NoRegion)?;
        if size == 0 || addr < r.base || end > r.edge {
            return Err(MapError::NoRegion);
        }
        let at = r.reserved.partition_point(|s| s.start < addr);
        if (at > 0 && r.reserved[at - 1].start + r.reserved[at - 1].size > addr)
            || (at < r.reserved.len() && end > r.reserved[at].start)
        {
            return Err(MapError::NoRegion);
        }
        r.reserved
            .try_reserve(1)
            .map_err(|_| MapError::OutOfMemory)?;
        let id = next_reservation_id();
        r.reserved.insert(
            at,
            Slot {
                start: addr,
                size,
                ticket: 0,
                id,
            },
        );
        Ok(id)
    }

    pub(crate) fn split(
        &mut self,
        id: RegionId,
        addr: usize,
        size: usize,
        reservation_id: usize,
        left_size: usize,
    ) -> Result<(usize, usize, usize), MapError> {
        let right = addr.checked_add(left_size).ok_or(MapError::NoRegion)?;
        if left_size == 0
            || left_size >= size
            || !left_size.is_multiple_of(crate::memory::PAGE_SIZE)
            || !size.is_multiple_of(crate::memory::PAGE_SIZE)
        {
            return Err(MapError::NotAligned);
        }
        let r = self.region_mut(id).ok_or(MapError::NoRegion)?;
        let at = r
            .reserved
            .iter()
            .position(|s| {
                s.start == addr && s.size == size && s.ticket == 0 && s.id == reservation_id
            })
            .ok_or(MapError::NoRegion)?;
        r.reserved
            .try_reserve(1)
            .map_err(|_| MapError::OutOfMemory)?;
        let left_id = next_reservation_id();
        let right_id = next_reservation_id();
        r.reserved[at] = Slot {
            start: addr,
            size: left_size,
            ticket: 0,
            id: left_id,
        };
        r.reserved.insert(
            at + 1,
            Slot {
                start: right,
                size: size - left_size,
                ticket: 0,
                id: right_id,
            },
        );
        Ok((right, left_id, right_id))
    }

    pub(crate) fn release(
        &mut self,
        id: RegionId,
        addr: usize,
        size: usize,
        reservation_id: usize,
    ) -> bool {
        let Some(r) = self.region_mut(id) else {
            return false;
        };
        match r.reserved.iter().position(|s| {
            s.start == addr && s.size == size && s.ticket == 0 && s.id == reservation_id
        }) {
            Some(at) => {
                r.reserved.remove(at);
                self.cleanups.retain(|(id, _)| *id != reservation_id);
                true
            }
            None => false,
        }
    }

    pub(crate) fn prepare_cleanup(&mut self, reservation_id: usize) -> Result<(), MapError> {
        if self.cleanups.iter().any(|(id, _)| *id == reservation_id) {
            return Ok(());
        }
        self.cleanups
            .try_reserve(1)
            .map_err(|_| MapError::OutOfMemory)?;
        let node =
            Box::try_new(super::space::CleanupNode::empty()).map_err(|_| MapError::OutOfMemory)?;
        self.cleanups.push((reservation_id, node));
        Ok(())
    }

    pub(crate) fn take_cleanup(
        &mut self,
        reservation_id: usize,
    ) -> Option<Box<super::space::CleanupNode>> {
        self.cleanups
            .iter()
            .position(|(id, _)| *id == reservation_id)
            .map(|at| self.cleanups.swap_remove(at).1)
    }

    pub(crate) fn exact_reservation(&self, addr: usize, size: usize) -> Option<(RegionId, usize)> {
        self.regions.iter().find_map(|r| {
            r.reserved
                .iter()
                .find(|s| s.start == addr && s.size == size && s.ticket == 0)
                .map(|s| (r.id, s.id))
        })
    }

    pub(crate) fn has_id(&self, region: RegionId, id: usize) -> bool {
        self.region(region)
            .is_some_and(|region| region.reserved.iter().any(|slot| slot.id == id))
    }

    pub(crate) fn cuts(
        &self,
        region: RegionId,
        start: usize,
        size: usize,
    ) -> Result<Vec<(usize, usize, usize)>, MapError> {
        let end = start.checked_add(size).ok_or(MapError::NotAligned)?;
        let region = self.region(region).ok_or(MapError::NoRegion)?;
        let mut cuts = Vec::new();
        cuts.try_reserve(region.reserved.len())
            .map_err(|_| MapError::OutOfMemory)?;
        for slot in &region.reserved {
            let lo = start.max(slot.start);
            let hi = end.min(slot.start + slot.size);
            if lo < hi {
                if slot.ticket != 0 {
                    return Err(MapError::Shootdown);
                }
                cuts.push((lo, hi - lo, slot.id));
            }
        }
        Ok(cuts)
    }

    pub(crate) fn reserve_cuts(&mut self, region: RegionId, count: usize) -> Result<(), MapError> {
        self.region_mut(region)
            .ok_or(MapError::NoRegion)?
            .reserved
            .try_reserve(count)
            .map_err(|_| MapError::OutOfMemory)
    }

    pub(crate) fn retire(
        &mut self,
        id: RegionId,
        addr: usize,
        size: usize,
        reservation_id: usize,
    ) -> usize {
        static NEXT: AtomicUsize = AtomicUsize::new(1);
        let ticket = NEXT.fetch_add(1, Ordering::Relaxed);
        assert_ne!(ticket, 0, "address retirement exhausted");
        let end = addr + size;
        let r = self.region_mut(id).expect("retired address region");
        let mut i = 0;
        while i < r.reserved.len() {
            let slot = &r.reserved[i];
            let (start, len, old_ticket, old_id) = (slot.start, slot.size, slot.ticket, slot.id);
            let stop = start + len;
            if old_ticket != 0 || old_id != reservation_id || addr >= stop || end <= start {
                i += 1;
                continue;
            }
            r.reserved.remove(i);
            if start < addr {
                r.reserved.insert(
                    i,
                    Slot {
                        start,
                        size: addr - start,
                        ticket: 0,
                        id: old_id,
                    },
                );
                i += 1;
            }
            let lo = start.max(addr);
            let hi = stop.min(end);
            r.reserved.insert(
                i,
                Slot {
                    start: lo,
                    size: hi - lo,
                    ticket,
                    id: old_id,
                },
            );
            i += 1;
            if stop > end {
                r.reserved.insert(
                    i,
                    Slot {
                        start: end,
                        size: stop - end,
                        ticket: 0,
                        id: old_id,
                    },
                );
                i += 1;
            }
        }
        ticket
    }

    pub(crate) fn reclaim(&mut self, id: RegionId, ticket: usize) {
        let removed = self
            .region(id)
            .and_then(|r| r.reserved.iter().find(|s| s.ticket == ticket).map(|s| s.id));
        if let Some(r) = self.region_mut(id) {
            r.reserved.retain(|s| s.ticket != ticket);
        }
        if let Some(reservation_id) = removed {
            if !self
                .regions
                .iter()
                .any(|r| r.reserved.iter().any(|s| s.id == reservation_id))
            {
                self.cleanups.retain(|(id, _)| *id != reservation_id);
            }
        }
    }

    fn region(&self, id: RegionId) -> Option<&Region> {
        self.regions.iter().find(|r| r.id == id)
    }
    fn region_mut(&mut self, id: RegionId) -> Option<&mut Region> {
        self.regions.iter_mut().find(|r| r.id == id)
    }
}

fn next_reservation_id() -> usize {
    static NEXT: AtomicUsize = AtomicUsize::new(1);
    let id = NEXT.fetch_add(1, Ordering::Relaxed);
    assert_ne!(id, 0, "address reservation identities exhausted");
    id
}
