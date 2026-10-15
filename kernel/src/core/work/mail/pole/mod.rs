mod authority;
mod held_map;
mod pole_operation;
mod relation;

use alloc::sync::{Arc, Weak};
use alloc::vec::Vec;
use core::sync::atomic::{AtomicBool, Ordering};
use core::time::Duration;

use crate::core::memory::PAGE_SIZE;
use crate::core::memory::address::{PageCount, PageOffset, PhysAddr};
use crate::core::memory::flat::{BorrowedRegion, Flat, FlatError, FlatRange};
use crate::core::memory::nest::{Access, NestError, View};
use crate::core::work::grant::Grant;
use crate::core::work::room::messenger::{self, Handoff, WakeKey};
use crate::core::work::unit::life::Life;
use crate::core::work::unit::space::{Mapping, Space};
use crate::machine::sync::{Level, SpinLock};
use env::{Bit, MailFail, Permission, PieFail, PieToken, TaskId, TeamId};

use authority::Authority;
use held_map::HeldMap;
use relation::Relation;

pub(crate) use pole_operation::PoleOperation;

/// 资源授权：一段经过授权的来源范围，它的授权记录与活跃关系。
///
/// 来源由 `source` 唯一承担：保留 RAM、设备区间与后续挂载都从这里出发，
/// 不为持有内存另建 kernel VA，也没有 keeper / canonical 来源视图。
/// 授权记录（token → 权限位）与实际 `View` 分开保存；
/// 该 token 下的关系自带 token，所以撤销边不丢。
pub struct PoleMeta {
    source: FlatRange,
    maps: SpinLock<Vec<HeldMap>>,
    authorities: SpinLock<Vec<Authority>>,
    live: AtomicBool,
    committed: AtomicBool,
    operating: AtomicBool,
    owner: TaskId,
    pub(crate) id: usize,
    life: Arc<Life>,
    ring: SpinLock<usize>,
}

unsafe impl Send for PoleMeta {}
unsafe impl Sync for PoleMeta {}

impl PoleMeta {
    fn new(source: FlatRange, size: usize, owner: TaskId) -> Result<Arc<Self>, PieFail> {
        if source.page_count().checked_bytes() != Some(size) {
            return Err(PieFail::Denied);
        }
        let mut maps = Vec::new();
        maps.try_reserve_exact(1).map_err(|_| PieFail::OoM)?;
        let mut authorities = Vec::new();
        authorities.try_reserve_exact(1).map_err(|_| PieFail::OoM)?;
        Arc::try_new(Self {
            source,
            maps: SpinLock::new(maps),
            authorities: SpinLock::new_level(Level::L3, authorities),
            live: AtomicBool::new(true),
            committed: AtomicBool::new(false),
            operating: AtomicBool::new(false),
            owner,
            id: super::next_id(),
            life: Life::try_new().map_err(|_| PieFail::OoM)?,
            ring: SpinLock::new_level(Level::L3, 0),
        })
        .map_err(|_| PieFail::OoM)
    }

    /// 从一段完整、已发布的现有映射建立输入来源。
    ///
    /// 来源范围归 Pole 持有；原映射作为该 token 下的活跃关系保留，
    /// 于是它被关闭后 RAM 仍由来源范围保留。
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
        let meta = Self::new(mapping.view().source(), size, owner)?;
        meta.add(mapping, space, None, Relation::Input);
        Ok(meta)
    }

    /// 接管输入映射的关闭责任。
    pub(crate) fn commit_source(&self) {
        self.committed.store(true, Ordering::Release);
    }
    pub(crate) fn owner(&self) -> TaskId {
        self.owner
    }
    pub(crate) fn size(&self) -> usize {
        self.source
            .page_count()
            .checked_bytes()
            .expect("validated Pole source size")
    }
    /// 授权的来源范围。普通 clone 只是强持同一 `Flat`，不产生独立 Hold。
    pub(crate) fn source(&self) -> FlatRange {
        self.source.clone()
    }
    /// 授权范围内的一段页对齐子范围；越界或非页对齐都拒。
    pub(crate) fn window(&self, offset: usize, size: usize) -> Result<FlatRange, PieFail> {
        if size == 0
            || !offset.is_multiple_of(PAGE_SIZE)
            || !size.is_multiple_of(PAGE_SIZE)
            || offset.checked_add(size).is_none_or(|end| end > self.size())
        {
            return Err(PieFail::NotAligned);
        }
        if offset == 0 && size == self.size() {
            return Ok(self.source());
        }
        let count = PageCount::new(size / PAGE_SIZE).ok_or(PieFail::NotAligned)?;
        self.source
            .range(PageOffset::new(offset / PAGE_SIZE), count)
            .map_err(|_| PieFail::NotAligned)
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

    /// 来源是否完整：只有 Managed 的整段物化才算自有，借用来源没有物化页槽。
    ///
    /// 只读快照：不物化、不分配。
    pub(crate) fn owned(&self) -> bool {
        self.source.fully_materialized().unwrap_or(false)
    }

    /// 登记一份授权：首个授权同时接管输入映射，它成为该 token 下的活跃关系。
    ///
    /// 两个集合的容量先各自预留，之后不再失败；授权与放置由 Pole 操作租约串行化。
    pub(crate) fn register_grant(&self, token: PieToken, grant: Arc<Grant>) -> Result<(), PieFail> {
        self.authorities
            .lock()
            .try_reserve(1)
            .map_err(|_| PieFail::OoM)?;
        self.maps.lock().try_reserve(1).map_err(|_| PieFail::OoM)?;
        {
            let mut maps = self.maps.lock();
            if let Some(input) = maps
                .iter_mut()
                .find(|entry| entry.input() && entry.token.is_none())
            {
                input.token = Some(token);
            }
        }
        self.authorities.lock().push(Authority::new(token, grant));
        Ok(())
    }

    pub(crate) fn release_grant(&self, token: PieToken) {
        self.authorities
            .lock()
            .retain(|record| !record.authorizes(token));
    }

    pub(crate) fn grant(&self, token: PieToken) -> Option<Arc<Grant>> {
        self.authorities
            .lock()
            .iter()
            .find(|record| record.authorizes(token))
            .map(Authority::grant)
    }

    pub(crate) fn readonly(&self) -> bool {
        if self.authorities.lock().iter().any(Authority::stores) {
            return false;
        }
        self.maps.lock().iter().all(|entry| !entry.writable())
    }

    pub(crate) fn exclusive(&self) -> bool {
        self.authorities
            .lock()
            .iter()
            .filter(|record| record.permitted())
            .count()
            == 1
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
        relation: Relation,
    ) {
        self.maps
            .lock()
            .push(HeldMap::placed(mapping, space, token, relation));
    }

    /// 把一段已落地的关系记入该 token：先核对授权记录，再入活跃 View 表。
    pub(crate) fn record(
        &self,
        token: PieToken,
        space: &Arc<Space>,
        mapping: Mapping,
        staged: Option<TeamId>,
        opened: bool,
    ) -> Result<(), Mapping> {
        let permitted = self
            .authorities
            .lock()
            .iter()
            .any(|record| record.authorizes(token) && record.permitted());
        if !self.alive() || !permitted {
            return Err(mapping);
        }
        self.maps.lock().push(HeldMap::placed(
            mapping,
            space,
            Some(token),
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

/// 来源自己的失败码折成资源授权码，沿用地址空间路径的同一条折叠。
fn pie_source_error(error: FlatError) -> PieFail {
    pie_map_error(NestError::from(error))
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
        return Ok((view.initial_range().0.as_usize(), meta.size()));
    }
    meta.reserve_record()?;
    // 来源已在手上：选址之后直接按来源范围挂载，不再经过任何 kernel / canonical 视图。
    let mapping = space
        .memory
        .map_range(meta.source(), PAGE_SIZE, None, access, access)
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
    Ok((address, meta.size()))
}

pub(crate) fn shut(meta: &Arc<PoleMeta>, token: PieToken) -> Result<(), PieFail> {
    let _operation = PoleOperation::acquire(meta).ok_or(PieFail::Busy)?;
    meta.close_token(token)
}

pub(crate) fn narrow(meta: &Arc<PoleMeta>, token: PieToken, access: Access) -> Result<(), PieFail> {
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

/// 设备区域：借用的物理区间代替存储保留。
///
/// 既不清零、不按 RAM 回收，也不占内核 VA；实际 CPU 访问另建普通 `View`。
pub(crate) fn region(base: usize, reg: usize, owner: TaskId) -> Result<Arc<PoleMeta>, PieFail> {
    let end = base
        .checked_add(reg)
        .filter(|_| reg != 0)
        .ok_or(PieFail::NotAligned)?;
    let lo = base & !(PAGE_SIZE - 1);
    let hi = end
        .checked_next_multiple_of(PAGE_SIZE)
        .ok_or(PieFail::NotAligned)?;
    let size = hi - lo;
    let region =
        unsafe { BorrowedRegion::new(PhysAddr::from_raw(lo), size) }.map_err(pie_source_error)?;
    let source = Flat::from_region(region).map_err(pie_source_error)?.whole();
    PoleMeta::new(source, size, owner)
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
