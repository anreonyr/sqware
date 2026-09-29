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

pub struct Segments<'a> {
    space: &'a Space,
    va: usize,
    end: usize,
}

impl Iterator for Segments<'_> {
    type Item = (crate::memory::manager::addr::PhysAddr, PteFlags, usize);

    fn next(&mut self) -> Option<Self::Item> {
        if self.va >= self.end {
            return None;
        }
        let (pa, flags) = self.space.translate(VirtAddr::from_raw(self.va))?;
        let page = self.va & !(PAGE_SIZE - 1);
        let chunk = (page + PAGE_SIZE - self.va).min(self.end - self.va);
        self.va += chunk;
        Some((pa, flags, chunk))
    }
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
        let mut space = Space {
            kind: self.kind,
            asid: self.asid,
            life: Life::new(),
            inner: RelLock::new_level(Level::Space, SpaceInner::durable()?),
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

    pub(crate) fn map(
        &self,
        va: VirtAddr,
        size: usize,
        flags: PteFlags,
        pending: Option<Pending>,
    ) -> Result<(), MapError> {
        let flags = self.pte_policy(flags);
        self.with(|inner| inner.map(va, size, flags, pending))
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

    pub(crate) fn release(&self, span: Span) -> Result<(), MapError> {
        let mut salvage = Salvage::new();
        self.with_flush(|inner| {
            if !inner.holds(span.seg, span.va.as_usize(), span.size.get()) {
                return Err(MapError::SegmentMismatch);
            }
            inner.unmap(span.va, span.size.get(), &mut salvage)?;
            salvage.take_span(span);
            Ok(())
        })?;
        salvage.reclaim(self).expect("release: shootdown deaf");
        Ok(())
    }

    pub fn materialize(&self, vaddr: VirtAddr, size: usize) -> Result<(), MapError> {
        self.with_flush(|inner| inner.materialize(vaddr, size))
    }

    pub fn protect(&self, vaddr: VirtAddr, size: usize, flags: PteFlags) -> Result<(), MapError> {
        let flags = self.pte_policy(flags);
        self.with_shootdown(|inner| inner.protect(vaddr, size, flags))
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

    pub fn segments(&self, va: VirtAddr, len: usize) -> Segments<'_> {
        Segments {
            space: self,
            va: va.as_usize(),
            end: va.as_usize() + len,
        }
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
