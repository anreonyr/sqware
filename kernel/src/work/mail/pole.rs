use alloc::sync::{Arc, Weak};
use alloc::vec::Vec;
use core::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use core::time::Duration;

use crate::lock::{Level, SpinLock};
use crate::memory::PAGE_SIZE;
use crate::memory::manager::{Map, MapError, addr::PhysAddr, entry::PteFlags};
use crate::work::room::messenger::{self, Handoff, WakeKey};
use crate::work::unit::gate::Grant;
use crate::work::unit::life::Life;
use crate::work::unit::space::{Mapping, RegionId, Reservation, Space};
use env::{Bit, MailFail, Permission, PieFail, PieToken, TaskId, TeamId};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct PoleId(pub usize);

fn alloc_id() -> PoleId {
    static NEXT: AtomicUsize = AtomicUsize::new(1);
    PoleId(NEXT.fetch_add(1, Ordering::Relaxed))
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Relation {
    Input,
    Keeper,
    Authority,
    Open,
    View,
    Staged(TeamId),
}

struct HeldMap {
    map: Arc<Map>,
    token: Option<PieToken>,
    grant: Option<Arc<Grant>>,
    placement: Option<(Weak<Space>, Reservation)>,
    relation: Relation,
}

pub struct PoleMeta {
    maps: SpinLock<Vec<HeldMap>>,
    live: AtomicBool,
    committed: AtomicBool,
    operating: AtomicBool,
    size: usize,
    owner: TaskId,
    id: PoleId,
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
            id: alloc_id(),
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
        let (start, size) = mapping.map.initial_range();
        if size == 0
            || start != mapping.reservation.start()
            || size != mapping.reservation.size()
            || mapping.map.frame_count() != size / PAGE_SIZE
        {
            return Err(PieFail::Denied);
        }
        let ranges = mapping.map.ranges().map_err(pie_map_error)?;
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
        let kernel = crate::work::unit::team::kernel()
            .ok_or(PieFail::Dead)?
            .space
            .clone();
        let reservation = kernel
            .allocate_high(RegionId::Resource, size)
            .map_err(pie_map_error)?;
        let rw = PteFlags::R | PteFlags::W;
        let keeper = kernel
            .derive(
                reservation,
                &mapping.map,
                0,
                rw,
                rw | PteFlags::A | PteFlags::D,
            )
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
    pub(crate) fn id(&self) -> PoleId {
        self.id
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

    pub(crate) fn canonical_map(&self) -> Option<Arc<Map>> {
        let maps = self.maps.lock();
        maps.iter()
            .filter(|entry| entry.relation == Relation::Keeper)
            .chain(maps.iter().filter(|entry| entry.placement.is_some()))
            .find(|entry| entry.map.is_live())
            .map(|entry| entry.map.clone())
    }

    pub(crate) fn owned(&self) -> bool {
        self.canonical_map()
            .is_some_and(|map| map.frame_count() == self.size / PAGE_SIZE)
    }

    pub(crate) fn register_grant(&self, token: PieToken, grant: Arc<Grant>) -> Result<(), PieFail> {
        let source = self.canonical_map().ok_or(PieFail::Dead)?;
        let mut maps = self.maps.lock();
        maps.try_reserve(1).map_err(|_| PieFail::OoM)?;
        if let Some(input) = maps
            .iter_mut()
            .find(|entry| entry.relation == Relation::Input && entry.token.is_none())
        {
            input.token = Some(token);
            input.grant = Some(grant.clone());
        }
        maps.push(HeldMap {
            map: source,
            token: Some(token),
            grant: Some(grant),
            placement: None,
            relation: Relation::Authority,
        });
        Ok(())
    }

    pub(crate) fn release_grant(&self, token: PieToken) {
        self.maps
            .lock()
            .retain(|entry| !(entry.token == Some(token) && entry.relation == Relation::Authority));
    }

    pub(crate) fn grant(&self, token: PieToken) -> Option<Arc<Grant>> {
        self.maps
            .lock()
            .iter()
            .find(|entry| entry.relation == Relation::Authority && entry.token == Some(token))
            .and_then(|entry| entry.grant.clone())
    }

    pub(crate) fn readonly(&self) -> bool {
        let maps = self.maps.lock();
        for entry in maps.iter() {
            if entry.relation == Relation::Authority
                && entry
                    .grant
                    .as_ref()
                    .and_then(|grant| grant.permission())
                    .is_some_and(|permission| permission.contains(Permission::STORE))
            {
                return false;
            }
            if entry.placement.is_some()
                && entry.relation != Relation::Keeper
                && entry.map.is_live()
                && entry.map.ranges().map_or(true, |ranges| {
                    ranges
                        .iter()
                        .any(|range| range.current.contains(PteFlags::W))
                })
            {
                return false;
            }
        }
        true
    }

    pub(crate) fn exclusive(&self) -> bool {
        self.maps
            .lock()
            .iter()
            .filter(|entry| {
                entry.relation == Relation::Authority
                    && entry
                        .grant
                        .as_ref()
                        .and_then(|grant| grant.permission())
                        .is_some()
            })
            .count()
            == 1
    }

    pub(crate) fn reserved(&self) -> usize {
        self.maps
            .lock()
            .iter()
            .find_map(|entry| {
                if let Relation::Staged(team) = entry.relation {
                    Some(team.get())
                } else {
                    None
                }
            })
            .unwrap_or(0)
    }

    pub(crate) fn opened(&self, map: &Arc<Map>) -> bool {
        self.maps.lock().iter().any(|entry| {
            matches!(entry.relation, Relation::Input | Relation::Open)
                && Arc::ptr_eq(&entry.map, map)
                && entry.map.is_live()
        })
    }

    pub(crate) fn mapped(&self) -> bool {
        self.maps.lock().iter().any(|entry| {
            matches!(
                entry.relation,
                Relation::Input | Relation::Open | Relation::View
            ) && entry.map.is_live()
        })
    }

    pub(crate) fn operation(self: &Arc<Self>) -> Option<PoleOperation> {
        self.operating
            .compare_exchange(false, true, Ordering::Acquire, Ordering::Relaxed)
            .ok()?;
        Some(PoleOperation(self.clone()))
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
        self.maps.lock().push(HeldMap {
            map: mapping.map,
            token,
            grant,
            placement: Some((Arc::downgrade(space), mapping.reservation)),
            relation,
        });
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
            .find(|entry| entry.relation == Relation::Authority && entry.token == Some(token))
            .and_then(|entry| entry.grant.clone());
        if !self.alive()
            || grant
                .as_ref()
                .and_then(|grant| grant.permission())
                .is_none()
        {
            return Err(mapping);
        }
        maps.push(HeldMap {
            map: mapping.map,
            token: Some(token),
            grant,
            placement: Some((Arc::downgrade(space), mapping.reservation)),
            relation: staged.map_or(
                if opened {
                    Relation::Open
                } else {
                    Relation::View
                },
                Relation::Staged,
            ),
        });
        Ok(())
    }

    pub(crate) fn detach_staging(&self, token: PieToken, team: TeamId) {
        self.maps.lock().retain(|entry| {
            !(entry.token == Some(token) && entry.relation == Relation::Staged(team))
        });
    }

    fn close_token(&self, token: PieToken) -> Result<(), PieFail> {
        loop {
            let mut entry = {
                let mut maps = self.maps.lock();
                let index = maps.iter().position(|entry| {
                    entry.token == Some(token)
                        && entry.placement.is_some()
                        && !matches!(entry.relation, Relation::Keeper | Relation::Staged(_))
                });
                match index {
                    Some(index) => maps.remove(index),
                    None => return Ok(()),
                }
            };
            if let Some((weak, reservation)) = entry.placement.take() {
                if let Some(space) = weak.upgrade() {
                    let mapping = Mapping {
                        map: entry.map.clone(),
                        reservation,
                    };
                    if let Err(error) = space.unmap(&mapping) {
                        entry.placement = Some((weak, mapping.reservation));
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
                let index = maps.iter().position(|entry| {
                    entry.token == Some(token)
                        && entry.placement.is_some()
                        && !matches!(entry.relation, Relation::Keeper | Relation::Staged(_))
                });
                match index {
                    Some(index) => maps.remove(index),
                    None => return,
                }
            };
            if let Some((weak, reservation)) = entry.placement {
                if let Some(space) = weak.upgrade() {
                    let _ = space.dispose(Mapping {
                        map: entry.map,
                        reservation,
                    });
                }
            }
        }
    }
}

pub(crate) struct PoleOperation(Arc<PoleMeta>);
impl Drop for PoleOperation {
    fn drop(&mut self) {
        self.0.operating.store(false, Ordering::Release);
    }
}

fn pie_map_error(error: MapError) -> PieFail {
    match error {
        MapError::OutOfMemory => PieFail::OoM,
        MapError::Shootdown => PieFail::Busy,
        _ => PieFail::Denied,
    }
}

impl Drop for PoleMeta {
    fn drop(&mut self) {
        self.live.store(false, Ordering::Release);
        wipe(self);
        let maps = core::mem::take(&mut *self.maps.lock());
        for entry in maps {
            if entry.relation == Relation::Input && !self.committed.load(Ordering::Acquire) {
                continue;
            }
            if let Some((weak, reservation)) = entry.placement {
                if let Some(space) = weak.upgrade() {
                    let _ = space.dispose(Mapping {
                        map: entry.map,
                        reservation,
                    });
                }
            }
        }
    }
}

pub(crate) fn open(
    meta: &Arc<PoleMeta>,
    token: PieToken,
    space: &Arc<Space>,
    flags: PteFlags,
) -> Result<(usize, usize), PieFail> {
    let _operation = meta.operation().ok_or(PieFail::Busy)?;
    if !meta.alive() {
        return Err(PieFail::Dead);
    }
    if meta.reserved() != 0 {
        return Err(PieFail::HandedOver);
    }
    let grant = meta.grant(token).ok_or(PieFail::Denied)?;
    let permission = grant.permission().ok_or(PieFail::Dead)?;
    if !permission.contains(Permission::FETCH)
        || flags.contains(PteFlags::W) && !permission.contains(Permission::STORE)
    {
        return Err(PieFail::Denied);
    }
    let reusable = {
        let maps = meta.maps.lock();
        maps.iter()
            .find(|entry| {
                entry.token == Some(token)
                    && matches!(entry.relation, Relation::Input | Relation::Open)
                    && entry
                        .placement
                        .as_ref()
                        .is_some_and(|(weak, _)| weak.ptr_eq(&Arc::downgrade(space)))
                    && entry.map.is_live()
            })
            .map(|entry| entry.map.clone())
    };
    if let Some(map) = reusable {
        for range in map.ranges().map_err(pie_map_error)? {
            let rwx = PteFlags::R | PteFlags::W | PteFlags::X;
            if (range.ceiling & rwx).bits() != (flags & rwx).bits() {
                space
                    .tree
                    .narrow(&map, range.start, range.size.get(), flags)
                    .map_err(pie_map_error)?;
            }
        }
        return Ok((map.initial_range().0.as_usize(), meta.size));
    }
    meta.reserve_record()?;
    let source = meta.canonical_map().ok_or(PieFail::Dead)?;
    let reservation = space
        .allocate_low(RegionId::Normal, meta.size)
        .map_err(pie_map_error)?;
    let mapping = space
        .derive(
            reservation,
            &source,
            0,
            flags,
            flags | PteFlags::A | PteFlags::D,
        )
        .map_err(pie_map_error)?;
    let address = mapping.reservation.start().as_usize();
    if !meta.alive() || grant.permission().is_none() {
        let _ = space.dispose(mapping);
        return Err(PieFail::Dead);
    }
    if let Err(mapping) = meta.record(token, space, mapping, None, true) {
        let _ = space.dispose(mapping);
        return Err(PieFail::Dead);
    }
    Ok((address, meta.size))
}

pub(crate) fn shut(meta: &Arc<PoleMeta>, token: PieToken) -> Result<(), PieFail> {
    let _operation = meta.operation().ok_or(PieFail::Busy)?;
    meta.close_token(token)
}

pub(crate) fn narrow(
    meta: &Arc<PoleMeta>,
    token: PieToken,
    flags: PteFlags,
) -> Result<(), PieFail> {
    if !meta.alive() {
        return Err(PieFail::Dead);
    }
    let mut targets = Vec::new();
    {
        let maps = meta.maps.lock();
        targets.try_reserve(maps.len()).map_err(|_| PieFail::OoM)?;
        for entry in maps
            .iter()
            .filter(|entry| entry.token == Some(token) && entry.placement.is_some())
        {
            if let Some(tree) = entry.map.tree() {
                targets.push((
                    tree,
                    entry.map.clone(),
                    entry.map.ranges().map_err(pie_map_error)?,
                ));
            }
        }
    }
    for (tree, map, ranges) in targets {
        for range in ranges {
            tree.narrow(&map, range.start, range.size.get(), flags)
                .map_err(pie_map_error)?;
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
    let kernel = crate::work::unit::team::kernel()
        .ok_or(PieFail::Dead)?
        .space
        .clone();
    let reservation = kernel
        .allocate_high(RegionId::Resource, hi - lo)
        .map_err(pie_map_error)?;
    let rw = PteFlags::R | PteFlags::W;
    let mapping = unsafe {
        kernel.map_borrowed(
            reservation,
            PhysAddr::from_raw(lo),
            rw,
            rw | PteFlags::A | PteFlags::D,
        )
    }
    .map_err(pie_map_error)?;
    meta.add(mapping, &kernel, None, None, Relation::Keeper);
    meta.commit_source();
    Ok(meta)
}

pub(crate) fn key(meta: &PoleMeta, bit: Bit) -> WakeKey {
    WakeKey::Pole { id: meta.id.0, bit }
}
fn wipe(meta: &PoleMeta) {
    messenger::wipe(WakeKey::Seal {
        kind: env::PieKind::Pole as u8,
        id: meta.id.0,
    });
    for index in 0..usize::BITS as usize {
        messenger::wipe(key(meta, Bit::of(index).unwrap()));
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
