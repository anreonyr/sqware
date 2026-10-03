use alloc::sync::{Arc, Weak};
use alloc::vec::Vec;

use super::SpaceKind;
use super::inner::SpaceInner;
use super::map::{Pending, PendingState};
use super::salvage::{Salvage, Span};
use crate::layout::TRAMPOLINE;
use crate::lock::{Level, RelLock};
use crate::memory::PAGE_SIZE;
use crate::memory::manager::MapError;
use crate::memory::manager::addr::{PhysAddr, VirtAddr};
use crate::memory::manager::asid::{self, Asid, Deaf};
use crate::memory::manager::entry::PteFlags;
use crate::memory::manager::flush_asid;
use crate::memory::manager::table::Frame;
use crate::work::unit::life::Life;

unsafe impl Send for Space {}
unsafe impl Sync for Space {}

pub struct Space {
    inner: RelLock<SpaceInner>,
    kind: SpaceKind,
    asid: Asid,
    life: Arc<Life>,
}

pub struct SpaceBuilder {
    kind: SpaceKind,
    asid: Asid,
}

impl SpaceBuilder {
    pub fn kernel() -> Self {
        Self {
            kind: SpaceKind::Supervisor,
            asid: Asid::kernel(),
        }
    }

    pub fn supervisor() -> Self {
        Self {
            kind: SpaceKind::Supervisor,
            asid: Asid::allocate(),
        }
    }

    pub fn user() -> Self {
        Self {
            kind: SpaceKind::User,
            asid: Asid::allocate(),
        }
    }

    pub fn build(self) -> Result<Space, MapError> {
        let prepared = (|| Ok::<_, MapError>((Life::try_new()?, SpaceInner::durable()?)))();
        let (life, inner) = match prepared {
            Ok(prepared) => prepared,
            Err(error) => {
                if !self.asid.is_kernel() {
                    asid::deallocate(self.asid).expect("space prepare: shootdown");
                }
                return Err(error);
            }
        };
        let mut space = Space {
            kind: self.kind,
            asid: self.asid,
            life,
            inner: RelLock::new_level(Level::Space, inner),
        };
        if !self.asid.is_kernel() {
            self.seed_trampoline(&mut space)?;
        }
        Ok(space)
    }

    fn seed_trampoline(&self, space: &mut Space) -> Result<(), MapError> {
        let (tramp_pa, tramp_flags) = {
            let ks_inner = crate::work::unit::team::kernel()
                .expect("kernel team not initialized")
                .space
                .inner
                .lock();
            ks_inner.root.walk_ref(TRAMPOLINE)?
        };
        space.borrow(TRAMPOLINE, tramp_pa, PAGE_SIZE, tramp_flags)?;
        Ok(())
    }
}

impl Space {
    pub(crate) fn has_token(&self, token: env::PieToken) -> bool {
        self.with(|inner| inner.maps.iter().any(|item|
            matches!(&item.origin, super::map::Origin::Backed { token: Some(t), .. } if *t == token)))
    }

    pub fn kind(&self) -> SpaceKind {
        self.kind
    }

    pub fn pte_policy(&self, flags: PteFlags) -> PteFlags {
        if self.kind().is_supervisor() {
            flags - PteFlags::U
        } else {
            flags | PteFlags::U
        }
    }

    pub fn asid(&self) -> Asid {
        self.asid
    }

    pub fn life(&self) -> Weak<Life> {
        Arc::downgrade(&self.life)
    }

    pub fn root(&self) -> usize {
        self.with(|inner| inner.root.ppn())
    }

    pub(crate) fn with<R>(&self, op: impl FnOnce(&mut SpaceInner) -> R) -> R {
        let mut inner = self.inner.lock();
        let r = op(&mut inner);
        drop(inner);
        r
    }

    pub(crate) fn with_flush<R>(&self, op: impl FnOnce(&mut SpaceInner) -> R) -> R {
        let r = self.with(op);
        // SAFETY: 见 flush_asid
        unsafe {
            flush_asid(self.asid);
        }
        r
    }

    pub(crate) fn with_shootdown<R>(
        &self,
        op: impl FnOnce(&mut SpaceInner) -> R,
    ) -> Result<R, Deaf> {
        let r = self.with(op);
        asid::shootdown(self.asid)?;
        Ok(r)
    }

    pub fn borrow(
        &self,
        vaddr: VirtAddr,
        paddr: PhysAddr,
        size: usize,
        flags: PteFlags,
    ) -> Result<(), MapError> {
        self.with_flush(|inner| inner.borrow(vaddr, paddr, size, flags))
    }

    pub(crate) fn attach(
        &self,
        vaddr: VirtAddr,
        frames: Vec<Frame>,
        flags: PteFlags,
    ) -> Result<(), MapError> {
        let flags = self.pte_policy(flags);
        self.with_flush(|inner| inner.attach(vaddr, frames, flags))
    }

    pub fn unmap(&self, vaddr: VirtAddr, size: usize) -> Result<(), MapError> {
        let mut salvage = Salvage::new();
        let r = self.with_flush(|inner| inner.unmap(vaddr, size, &mut salvage));
        salvage.reclaim(self).expect("unmap: shootdown deaf");
        r
    }

    pub(crate) fn unmap_user(&self, addr: usize, size: usize) -> Result<(), MapError> {
        if !Self::user_range(addr, size) {
            return Err(MapError::NoRegion);
        }
        if !addr.is_multiple_of(PAGE_SIZE) || !size.is_multiple_of(PAGE_SIZE) {
            return Err(MapError::NotAligned);
        }
        let mut salvage = Salvage::new();
        let result = self.with_flush(|inner| {
            let va = VirtAddr::wrap(addr);
            let end = addr + size;
            let mut found = false;
            for map in inner.maps.overlapping(addr, end.saturating_sub(1)) {
                found = true;
                if !(map.pending == Some(Pending::Lazy)
                    || matches!(map.origin, super::map::Origin::Backed { open: false, .. }))
                {
                    return Err(MapError::NoRegion);
                }
            }
            if !found {
                return Err(MapError::NoRegion);
            }
            inner
                .user
                .as_mut()
                .ok_or(MapError::NoRegion)?
                .prepare_cut()
                .map_err(|_| MapError::OutOfMemory)?;
            inner.unmap(va, size, &mut salvage)?;
            Ok(inner
                .user
                .as_mut()
                .expect("normal segment")
                .retire(addr, size))
        });
        salvage.reclaim(self).expect("unmap: shootdown deaf");
        let ticket = result?;
        self.with(|inner| inner.user.as_mut().expect("normal segment").reclaim(ticket));
        Ok(())
    }

    pub(crate) fn release(&self, span: Span) -> Result<(), MapError> {
        self.release_if(span, |_| true)
    }

    pub(super) fn release_if(
        &self,
        span: Span,
        allowed: impl FnOnce(&SpaceInner) -> bool,
    ) -> Result<(), MapError> {
        let mut salvage = Salvage::new();
        self.with_flush(|inner| {
            if !allowed(inner) || !inner.holds(span.seg, span.va.as_usize(), span.size.get()) {
                return Err(MapError::SegmentMismatch);
            }
            inner.unmap(span.va, span.size.get(), &mut salvage)?;
            salvage.take_span(span);
            Ok(())
        })?;
        salvage.reclaim(self).expect("release: shootdown deaf");
        Ok(())
    }

    pub(crate) fn unmap_token(&self, token: env::PieToken) -> Result<(), MapError> {
        loop {
            let mut salvage = Salvage::new();
            let ticket = self.with_flush(|inner| {
                let at = inner.maps.iter().find(|map|
                    matches!(map.origin, super::map::Origin::Backed { token: Some(t), .. } if t == token))
                    .map(|map| map.va.as_usize());
                let Some(at) = at else { return Ok(None) };
                let (start, size) = inner.user.as_ref().and_then(|segment| segment.covering(at))
                    .ok_or(MapError::SegmentMismatch)?;
                // Protection may split Maps; the reservation still identifies the entire token view.
                inner.unmap(VirtAddr::wrap(start), size, &mut salvage)?;
                Ok(Some(inner.user.as_mut().expect("token segment").retire(start, size)))
            })?;
            salvage
                .reclaim(self)
                .expect("token unmap: shootdown failed");
            let Some(ticket) = ticket else {
                return Ok(());
            };
            self.with(|inner| inner.user.as_mut().expect("token segment").reclaim(ticket));
        }
    }

    pub fn materialize(&self, vaddr: VirtAddr, size: usize) -> Result<(), MapError> {
        self.with_flush(|inner| inner.materialize(vaddr, size))
    }

    pub fn protect(&self, vaddr: VirtAddr, size: usize, flags: PteFlags) -> Result<(), MapError> {
        let flags = self.pte_policy(flags);
        self.with_shootdown(|inner| inner.protect(vaddr, size, flags, false))
            .expect("protect: shootdown deaf")
    }

    pub(crate) fn protect_user(&self, addr: usize, size: usize, bits: u64) -> Result<(), MapError> {
        let rwx = PteFlags::R | PteFlags::W | PteFlags::X;
        let flags = PteFlags::from_bits(bits)
            .filter(|flags| {
                !flags.is_empty()
                    && rwx.contains(*flags)
                    && (!flags.contains(PteFlags::W) || flags.contains(PteFlags::R))
            })
            .ok_or(MapError::SegmentMismatch)?;
        if !Self::user_range(addr, size) {
            return Err(MapError::NoRegion);
        }
        if size == 0 || !size.is_multiple_of(PAGE_SIZE) || !addr.is_multiple_of(PAGE_SIZE) {
            return Err(MapError::NotAligned);
        }
        if flags.contains(PteFlags::X) {
            super::sync_instructions()?;
        }
        self.with_shootdown(|inner| {
            let va = VirtAddr::wrap(addr);
            if !inner.maps_in(va, size, |map| {
                map.pending != Some(Pending::Guard)
                    && !map.flags.contains(PteFlags::G)
                    && (self.kind().is_supervisor() || map.flags.contains(PteFlags::U))
            }) {
                return Err(MapError::NoRegion);
            }
            inner.protect(va, size, flags, true)
        })
        .expect("protect: shootdown deaf")
    }

    pub fn translate(&self, vaddr: VirtAddr) -> Option<(PhysAddr, PteFlags)> {
        self.with(|inner| inner.translate(vaddr))
    }

    pub fn pending_state(&self, vaddr: VirtAddr) -> PendingState {
        self.with(|inner| match inner.resolve_ref(vaddr) {
            Some(m) => match m.pending {
                None => PendingState::Materialized,
                Some(Pending::Lazy) => PendingState::Lazy,
                Some(Pending::Guard) => PendingState::Guard,
            },
            None => PendingState::Absent,
        })
    }

    #[cfg(debug_assertions)]
    pub fn table_count(&self) -> usize {
        self.with(|inner| inner.root.count())
    }

    #[cfg(debug_assertions)]
    pub(crate) fn audit(&self) {
        self.with(|inner| inner.audit());
    }
}

impl Drop for Space {
    fn drop(&mut self) {
        if !self.asid.is_kernel() {
            asid::deallocate(self.asid).expect("space drop: shootdown deaf");
        }
    }
}
