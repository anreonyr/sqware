mod held_map;
mod pole_operation;
mod relation;

use alloc::sync::{Arc, Weak};
use alloc::vec::Vec;
use core::sync::atomic::{AtomicBool, Ordering};
use core::time::Duration;

use crate::core::memory::PAGE_SIZE;
use crate::core::memory::address::{PageCount, PageOffset, PhysAddr};
use crate::core::memory::nest::{Access, NestError, RegionId, View};
use crate::core::work::grant::Grant;
use crate::core::work::room::messenger::{self, Handoff, WakeKey};
use crate::core::work::unit::life::Life;
use crate::core::work::unit::space::{Mapping, Space};
use crate::machine::sync::{Level, SpinLock};
use env::{Bit, MailFail, Permission, PieFail, PieToken, TaskId, TeamId};

use held_map::HeldMap;
use relation::Relation;

pub(crate) use pole_operation::PoleOperation;

pub struct PoleMeta {
    maps: SpinLock<Vec<HeldMap>>,
    live: AtomicBool,
    committed: AtomicBool,
    operating: AtomicBool,
    size: usize,
    owner: TaskId,
    pub(crate) id: usize,
    life: Arc<Life>,
    ring: SpinLock<usize>,
}

unsafe impl Send for PoleMeta {}
unsafe impl Sync for PoleMeta {}

impl PoleMeta {
    fn new(size: usize, owner: TaskId) -> Result<Arc<Self>, PieFail> {
        let mut maps = Vec::new();
        maps.try_reserve_exact(3).map_err(|_| PieFail::OoM)?;
        Arc::try_new(Self {
            maps: SpinLock::new(maps),
            live: AtomicBool::new(true),
            committed: AtomicBool::new(false),
            operating: AtomicBool::new(false),
            size,
            owner,
            id: super::next_id(),
            life: Life::try_new().map_err(|_| PieFail::OoM)?,
            ring: SpinLock::new_level(Level::L3, 0),
        })
        .map_err(|_| PieFail::OoM)
    }

    pub(crate) fn from_mapping(
        space: &Arc<Space>,
        mapping: Mapping,
        owner: TaskId,
    ) -> Result<Arc<Self>, PieFail> {
        let (start, size) = mapping.view().initial_range();
        if size == 0 || mapping.view().frame_count() != size / PAGE_SIZE {
            return Err(PieFail::Denied);
        }
        let ranges = mapping.view().active().map_err(pie_map_error)?;
        let mut covered = 0;
        for range in ranges {
            if start.as_usize().checked_add(covered) != Some(range.start.as_usize()) {
                return Err(PieFail::Denied);
            }
            covered += range.size.get();
        }
        if covered != size {
            return Err(PieFail::Denied);
        }
        let meta = Self::new(size, owner)?;
        let kernel = crate::core::work::unit::team::kernel()
            .ok_or(PieFail::Dead)?
            .space
            .clone();
        let reservation = kernel
            .memory
            .allocate_high(RegionId::Resource, size)
            .map_err(pie_map_error)?;
        let rw = Access::READ | Access::WRITE;
        let keeper = kernel
            .memory
            .derive_into(reservation, mapping.view(), 0, rw, rw)
            .map_err(pie_map_error)?;
        meta.add(mapping, space, None, None, Relation::Input);
        meta.add(keeper, &kernel, None, None, Relation::Keeper);
        Ok(meta)
    }

    pub(crate) fn commit_source(&self) {
        self.committed.store(true, Ordering::Release);
    }
    pub(crate) fn owner(&self) -> TaskId {
        self.owner
    }
    pub(crate) fn size(&self) -> usize {
        self.size
    }
    pub(crate) fn life(&self) -> Weak<Life> {
        Arc::downgrade(&self.life)
    }
    pub(crate) fn alive(&self) -> bool {
        self.live.load(Ordering::Acquire)
    }
    pub(crate) fn ready(&self, bit: Bit) -> bool {
        *self.ring.lock() & bit.mask() != 0
    }

    pub(crate) fn canonical_view(&self) -> Option<Arc<View>> {
        let maps = self.maps.lock();
        maps.iter()
            .filter(|entry| entry.keeper())
            .chain(maps.iter().filter(|entry| entry.placement.is_some()))
            .find(|entry| entry.live())
            .map(|entry| entry.view.clone())
    }

    pub(crate) fn owned(&self) -> bool {
        self.canonical_view()
            .is_some_and(|view| view.frame_count() == self.size / PAGE_SIZE)
    }

    pub(crate) fn register_grant(&self, token: PieToken, grant: Arc<Grant>) -> Result<(), PieFail> {
        let source = self.canonical_view().ok_or(PieFail::Dead)?;
        let mut maps = self.maps.lock();
        maps.try_reserve(1).map_err(|_| PieFail::OoM)?;
        if let Some(input) = maps
            .iter_mut()
            .find(|entry| entry.input() && entry.token.is_none())
        {
            input.token = Some(token);
            input.grant = Some(grant.clone());
        }
        maps.push(HeldMap::authority(source, token, grant));
        Ok(())
    }

    pub(crate) fn release_grant(&self, token: PieToken) {
        self.maps.lock().retain(|entry| !entry.authorizes(token));
    }

    pub(crate) fn grant(&self, token: PieToken) -> Option<Arc<Grant>> {
        self.maps
            .lock()
            .iter()
            .find(|entry| entry.authorizes(token))
            .and_then(|entry| entry.grant.clone())
    }

    pub(crate) fn readonly(&self) -> bool {
        let maps = self.maps.lock();
        for entry in maps.iter() {
            if entry.storing() || entry.writable() {
                return false;
            }
        }
        true
    }

    pub(crate) fn exclusive(&self) -> bool {
        let maps = self.maps.lock();
        maps.iter().filter(|entry| entry.granted()).count() == 1
    }

    pub(crate) fn reserved(&self) -> usize {
        self.maps
            .lock()
            .iter()
            .find_map(|entry| entry.staged())
            .map_or(0, TeamId::get)
    }

    pub(crate) fn opened(&self, view: &Arc<View>) -> bool {
        self.maps
            .lock()
            .iter()
            .any(|entry| entry.opened() && Arc::ptr_eq(&entry.view, view) && entry.live())
    }

    pub(crate) fn mapped(&self) -> bool {
        self.maps
            .lock()
            .iter()
            .any(|entry| entry.mapped() && entry.live())
    }

    pub(crate) fn reserve_record(&self) -> Result<(), PieFail> {
        self.maps.lock().try_reserve(1).map_err(|_| PieFail::OoM)
    }

    fn add(
        &self,
        mapping: Mapping,
        space: &Arc<Space>,
        token: Option<PieToken>,
        grant: Option<Arc<Grant>>,
        relation: Relation,
    ) {
        self.maps
            .lock()
            .push(HeldMap::placed(mapping, space, token, grant, relation));
    }

    pub(crate) fn record(
        &self,
        token: PieToken,
        space: &Arc<Space>,
        mapping: Mapping,
        staged: Option<TeamId>,
        opened: bool,
    ) -> Result<(), Mapping> {
        let mut maps = self.maps.lock();
        let grant = maps
            .iter()
            .find(|entry| entry.authorizes(token))
            .and_then(|entry| entry.grant.clone());
        if !self.alive()
            || grant
                .as_ref()
                .and_then(|grant| grant.permission())
                .is_none()
        {
            return Err(mapping);
        }
        maps.push(HeldMap::placed(
            mapping,
            space,
            Some(token),
            grant,
            staged.map_or(
                if opened {
                    Relation::Open
                } else {
                    Relation::View
                },
                Relation::Staged,
            ),
        ));
        Ok(())
    }

    pub(crate) fn detach_staging(&self, token: PieToken, team: TeamId) {
        self.maps
            .lock()
            .retain(|entry| !entry.staged_for(token, team));
    }

    fn close_token(&self, token: PieToken) -> Result<(), PieFail> {
        loop {
            let mut entry = {
                let mut maps = self.maps.lock();
                let index = maps.iter().position(|entry| entry.closable(token));
                match index {
                    Some(index) => maps.remove(index),
                    None => return Ok(()),
                }
            };
            if let Some(weak) = entry.placement.take() {
                if let Some(space) = weak.upgrade() {
                    let mapping = Mapping::new(entry.view.clone());
                    if let Err(error) = space.memory.unmap(&mapping) {
                        entry.placement = Some(weak);
                        self.maps.lock().push(entry);
                        return Err(pie_map_error(error));
                    }
                }
            }
        }
    }

    pub(crate) fn withdraw(&self, token: PieToken) {
        loop {
            let entry = {
                let mut maps = self.maps.lock();
                let index = maps.iter().position(|entry| entry.closable(token));
                match index {
                    Some(index) => maps.remove(index),
                    None => return,
                }
            };
            if let Some(weak) = entry.placement {
                if let Some(space) = weak.upgrade() {
                    let _ = space.memory.dispose(Mapping::new(entry.view));
                }
            }
        }
    }
}

fn pie_map_error(error: NestError) -> PieFail {
    match error {
        NestError::OutOfMemory => PieFail::OoM,
        NestError::Shootdown => PieFail::Busy,
        _ => PieFail::Denied,
    }
}

impl Drop for PoleMeta {
    fn drop(&mut self) {
        seal(self);
        let maps = core::mem::take(&mut *self.maps.lock());
        for entry in maps {
            if entry.input() && !self.committed.load(Ordering::Acquire) {
                continue;
            }
            if let Some(weak) = entry.placement {
                if let Some(space) = weak.upgrade() {
                    let _ = space.memory.dispose(Mapping::new(entry.view));
                }
            }
        }
    }
}

pub(crate) fn open(
    meta: &Arc<PoleMeta>,
    token: PieToken,
    space: &Arc<Space>,
    access: Access,
) -> Result<(usize, usize), PieFail> {
    let _operation = PoleOperation::acquire(meta).ok_or(PieFail::Busy)?;
    if !meta.alive() {
        return Err(PieFail::Dead);
    }
    if meta.reserved() != 0 {
        return Err(PieFail::HandedOver);
    }
    let grant = meta.grant(token).ok_or(PieFail::Denied)?;
    let permission = grant.permission().ok_or(PieFail::Dead)?;
    if !permission.contains(Permission::FETCH)
        || access.contains(Access::WRITE) && !permission.contains(Permission::STORE)
    {
        return Err(PieFail::Denied);
    }
    let reusable = {
        let maps = meta.maps.lock();
        maps.iter()
            .find(|entry| {
                entry.holds(token) && entry.opened() && entry.placed_in(space) && entry.live()
            })
            .map(|entry| entry.view.clone())
    };
    if let Some(view) = reusable {
        for range in view.active().map_err(pie_map_error)? {
            if range.ceiling != access {
                space
                    .memory
                    .narrow_record(&view, range.start, range.size.get(), access)
                    .map_err(pie_map_error)?;
            }
        }
        return Ok((view.initial_range().0.as_usize(), meta.size));
    }
    meta.reserve_record()?;
    let source = meta.canonical_view().ok_or(PieFail::Dead)?;
    let reservation = space
        .memory
        .allocate_low(RegionId::Normal, meta.size)
        .map_err(pie_map_error)?;
    let mapping = space
        .memory
        .derive_into(reservation, &source, 0, access, access)
        .map_err(pie_map_error)?;
    let address = mapping.address().as_usize();
    if !meta.alive() || grant.permission().is_none() {
        let _ = space.memory.dispose(mapping);
        return Err(PieFail::Dead);
    }
    if let Err(mapping) = meta.record(token, space, mapping, None, true) {
        let _ = space.memory.dispose(mapping);
        return Err(PieFail::Dead);
    }
    Ok((address, meta.size))
}

pub(crate) fn shut(meta: &Arc<PoleMeta>, token: PieToken) -> Result<(), PieFail> {
    let _operation = PoleOperation::acquire(meta).ok_or(PieFail::Busy)?;
    meta.close_token(token)
}

pub(crate) fn narrow(
    meta: &Arc<PoleMeta>,
    token: PieToken,
    access: Access,
) -> Result<(), PieFail> {
    if !meta.alive() {
        return Err(PieFail::Dead);
    }
    let mut targets = Vec::new();
    {
        let maps = meta.maps.lock();
        targets.try_reserve(maps.len()).map_err(|_| PieFail::OoM)?;
        for entry in maps.iter().filter(|entry| entry.placed_by(token)) {
            if let Some(nest) = entry.view.owner() {
                targets.push((
                    nest,
                    entry.view.clone(),
                    entry.view.active().map_err(pie_map_error)?,
                ));
            }
        }
    }
    for (nest, view, ranges) in targets {
        for range in ranges {
            let offset = (range.start.as_usize() - view.initial_start().as_usize()) / PAGE_SIZE;
            let count = PageCount::new(range.size.get() / PAGE_SIZE).ok_or(PieFail::NotAligned)?;
            let selection = view
                .range(PageOffset::new(offset), count)
                .map_err(pie_map_error)?;
            nest.narrow(selection, access).map_err(pie_map_error)?;
        }
    }
    Ok(())
}

pub(crate) fn region(base: usize, reg: usize, owner: TaskId) -> Result<Arc<PoleMeta>, PieFail> {
    let end = base
        .checked_add(reg)
        .filter(|_| reg != 0)
        .ok_or(PieFail::NotAligned)?;
    let lo = base & !(PAGE_SIZE - 1);
    let hi = end
        .checked_next_multiple_of(PAGE_SIZE)
        .ok_or(PieFail::NotAligned)?;
    let meta = PoleMeta::new(hi - lo, owner)?;
    let kernel = crate::core::work::unit::team::kernel()
        .ok_or(PieFail::Dead)?
        .space
        .clone();
    let reservation = kernel
        .memory
        .allocate_high(RegionId::Resource, hi - lo)
        .map_err(pie_map_error)?;
    let rw = Access::READ | Access::WRITE;
    let mapping = unsafe {
        kernel.memory.map_borrowed(
            reservation,
            PhysAddr::from_raw(lo),
            rw,
            rw,
        )
    }
    .map_err(pie_map_error)?;
    meta.add(mapping, &kernel, None, None, Relation::Keeper);
    meta.commit_source();
    Ok(meta)
}

pub(crate) fn key(meta: &PoleMeta, bit: Bit) -> WakeKey {
    WakeKey::Pole { id: meta.id, bit }
}
fn wipe(meta: &PoleMeta) {
    messenger::wipe(WakeKey::seal(env::PieKind::Pole, meta.id));
    for index in 0..usize::BITS as usize {
        messenger::wipe(key(meta, Bit::wrap(index)));
    }
}
pub(crate) fn ring(meta: &PoleMeta, bit: Bit) -> Result<(), MailFail> {
    if !meta.alive() {
        return Err(MailFail::Dead);
    }
    let changed = {
        let mut pending = meta.ring.lock();
        let changed = *pending & bit.mask() == 0;
        *pending |= bit.mask();
        changed
    };
    if changed {
        messenger::knock(key(meta, bit), &meta.life());
    }
    Ok(())
}
pub(crate) fn hush(meta: &PoleMeta, bit: Bit) -> Result<(), MailFail> {
    if !meta.alive() {
        return Err(MailFail::Dead);
    }
    *meta.ring.lock() &= !bit.mask();
    Ok(())
}
pub(crate) fn wait(meta: &PoleMeta, bit: Bit, dur: Duration) -> Result<Handoff<bool>, MailFail> {
    if !meta.alive() {
        return Err(MailFail::Dead);
    }
    if meta.ready(bit) {
        return Ok(Handoff::Resume(true));
    }
    if dur == Duration::ZERO {
        return Ok(Handoff::Resume(false));
    }
    Ok(match messenger::wait(key(meta, bit), meta.life(), dur)? {
        Handoff::Resume(()) => Handoff::Resume(meta.alive() && meta.ready(bit)),
        Handoff::Switch(pa) => Handoff::Switch(pa),
    })
}
pub(crate) fn seal(meta: &PoleMeta) {
    meta.live.store(false, Ordering::Release);
    wipe(meta);
}
