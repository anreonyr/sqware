use alloc::sync::Arc;
use alloc::vec::Vec;
use core::ptr::NonNull;

use env::TaskId;

use env::PieToken;

use crate::lock::{Level, SpinLock};
use crate::memory::PAGE_SIZE;
use crate::memory::allocator::frame;
use crate::memory::manager::MapError;
use crate::memory::manager::addr::{PhysAddr, VirtAddr};
use crate::memory::manager::entry::PteFlags;
use crate::work::unit::space::{SegmentKind, Space, Span};

use env::PieFail;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PoleState {
    Live,
    Dead,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Payload {
    Frames,
    Region,
}

pub struct PoleMeta {
    state: SpinLock<PoleState>,
    payload: Payload,
    base: NonNull<u8>,
    size: usize,
    mappings: SpinLock<Vec<(PieToken, alloc::sync::Weak<Space>, Span)>>,
    owner: TaskId,
}

// SAFETY: PoleMeta 经 Arc 跨任务共享；base 指向共享物理帧
unsafe impl Send for PoleMeta {}
unsafe impl Sync for PoleMeta {}

impl PoleMeta {
    pub(super) fn allocate(size: usize, owner: TaskId) -> Result<Arc<Self>, PieFail> {
        if size == 0 || !size.is_multiple_of(PAGE_SIZE) {
            return Err(PieFail::NotAligned);
        }
        let layout = core::alloc::Layout::from_size_align(size, PAGE_SIZE)
            .map_err(|_| PieFail::NotAligned)?;
        let ptr = crate::tag!(
            Pole,
            frame::allocator()
                .allocate(layout)
                .map_err(|_| PieFail::OoM)?
        );
        // SAFETY: 分配返回非空
        let base = unsafe { NonNull::new_unchecked(ptr.as_ptr().cast::<u8>()) };
        unsafe {
            core::ptr::write_bytes(base.as_ptr(), 0, size);
        }
        Ok(Arc::new(Self {
            state: SpinLock::new_level(Level::L3, PoleState::Live),
            payload: Payload::Frames,
            base,
            size,
            mappings: SpinLock::new(Vec::new()),
            owner,
        }))
    }

    pub(super) fn region(base: usize, reg: usize, owner: TaskId) -> Result<Arc<Self>, PieFail> {
        if reg == 0 {
            return Err(PieFail::NotAligned);
        }
        let end = base.checked_add(reg).ok_or(PieFail::NotAligned)?;
        let lo = base & !(PAGE_SIZE - 1);
        let hi = end.next_multiple_of(PAGE_SIZE);
        let base = NonNull::new(lo as *mut u8).ok_or(PieFail::NotAligned)?;
        let size = hi - lo;
        Ok(Arc::new(Self {
            state: SpinLock::new_level(Level::L3, PoleState::Live),
            payload: Payload::Region,
            base,
            size,
            mappings: SpinLock::new(Vec::new()),
            owner,
        }))
    }

    pub(crate) fn owner(&self) -> TaskId {
        self.owner
    }

    pub(crate) fn alive(&self) -> bool {
        *self.state.lock() == PoleState::Live
    }

    fn open_into(
        &self,
        token: PieToken,
        space: &Arc<Space>,
        flags: PteFlags,
    ) -> Result<usize, PieFail> {
        {
            let m = self.mappings.lock();
            if let Some((_, _, span)) = m.iter().find(|(t, _, _)| *t == token) {
                return Ok(span.va.as_usize());
            }
        }
        let va = space
            .with_flush(|inner| {
                let va = inner.allocate(SegmentKind::Normal, self.size)?;
                if let Err(e) = inner.borrow(
                    va,
                    PhysAddr::from_raw(self.base.as_ptr() as usize),
                    self.size,
                    flags,
                ) {
                    inner.deallocate(SegmentKind::Normal, va.as_usize(), self.size);
                    return Err(e);
                }
                Ok::<_, MapError>(va)
            })
            .map_err(|_| PieFail::OoM)?;
        let mut maps = self.mappings.lock();
        if maps.try_reserve(1).is_err() {
            drop(maps);
            let _ = space.release(Span::new(SegmentKind::Normal, va, self.size, None));
            return Err(PieFail::OoM);
        }
        maps.push((
            token,
            Arc::downgrade(space),
            Span::new(SegmentKind::Normal, va, self.size, None),
        ));
        Ok(va.as_usize())
    }

    fn narrow_into(&self, token: PieToken, flags: PteFlags) -> Result<(), PieFail> {
        let target = {
            let m = self.mappings.lock();
            m.iter()
                .find(|(t, _, _)| *t == token)
                .and_then(|(_, w, s)| {
                    w.upgrade()
                        .map(|space| (space, s.va.as_usize(), s.size.get()))
                })
        };
        if let Some((space, va, size)) = target {
            space
                .protect(VirtAddr::from_raw(va), size, flags)
                .map_err(|_| PieFail::Denied)?;
        }
        Ok(())
    }

    fn shut_from(&self, token: PieToken) -> Result<(), PieFail> {
        let (space, span) = {
            let mut m = self.mappings.lock();
            let pos = m.iter().position(|(t, _, _)| *t == token);
            match pos {
                Some(i) => {
                    let (_, w, s) = m.remove(i);
                    match w.upgrade() {
                        Some(space) => (space, s),
                        None => return Ok(()),
                    }
                }
                None => return Ok(()),
            }
        };
        space.release(span).map_err(|_| PieFail::Denied)
    }
}

impl Drop for PoleMeta {
    fn drop(&mut self) {
        *self.state.lock() = PoleState::Dead;
        let mappings: Vec<(PieToken, alloc::sync::Weak<Space>, Span)> =
            core::mem::take(&mut *self.mappings.lock());
        for (_, weak, span) in mappings {
            if let Some(space) = weak.upgrade() {
                let _ = space.release(span);
            }
        }
        let layout =
            core::alloc::Layout::from_size_align(self.size, PAGE_SIZE).expect("pole layout valid");
        if self.payload == Payload::Frames {
            unsafe {
                frame::allocator().deallocate(self.base, layout);
            }
        }
    }
}

pub(crate) fn open(
    meta: &PoleMeta,
    token: PieToken,
    space: &Arc<Space>,
    flags: PteFlags,
) -> Result<(usize, usize), PieFail> {
    if !meta.alive() {
        return Err(PieFail::Dead);
    }
    let va = meta.open_into(token, space, flags)?;
    let _ = space.protect(VirtAddr::from_raw(va), meta.size, flags);
    Ok((va, meta.size))
}

pub(crate) fn shut(meta: &PoleMeta, token: PieToken) -> Result<(), PieFail> {
    meta.shut_from(token)
}

pub(crate) fn narrow(meta: &PoleMeta, token: PieToken, flags: PteFlags) -> Result<(), PieFail> {
    if !meta.alive() {
        return Err(PieFail::Dead);
    }
    meta.narrow_into(token, flags)
}

pub(crate) fn seal(meta: &PoleMeta) {
    *meta.state.lock() = PoleState::Dead;
}

pub(crate) fn meta(size: usize, owner: TaskId) -> Result<Arc<PoleMeta>, PieFail> {
    PoleMeta::allocate(size, owner)
}

pub(crate) fn region(base: usize, reg: usize, owner: TaskId) -> Result<Arc<PoleMeta>, PieFail> {
    PoleMeta::region(base, reg, owner)
}
