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
use crate::core::memory::nest::{Access, NestError, Placement, RegionId, View};
use crate::core::work::grant::Grant;
use crate::core::work::pend::SourceAuthorization;
use crate::core::work::room::messenger::{self, Handoff, WakeKey};
use crate::core::work::unit::life::Life;
use crate::core::work::unit::space::Space;
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
    /// 这份来源的授权记录：从已有来源创建时就是那一份（根 Grant 接手），
    /// 否则为 `None`。撤销时由它收回名下关系。
    authorization: SpinLock<Option<Arc<SourceAuthorization>>>,
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
            authorization: SpinLock::new(None),
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

    /// 从一份**完整来源范围**建立 Pole：不造 keeper / canonical 来源视图，
    /// 真实关系由调用方（创建事务）决定什么时候登进来。
    pub(crate) fn from_source(
        source: FlatRange,
        size: usize,
        owner: TaskId,
    ) -> Result<Arc<Self>, PieFail> {
        if source.page_count().checked_bytes() != Some(size) {
            return Err(PieFail::Denied);
        }
        Self::new(source, size, owner)
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

    /// 这份 Pole 的来源授权记录；旧路径建出来的 Pole 没有。
    pub(crate) fn authorization(&self) -> Option<Arc<SourceAuthorization>> {
        self.authorization.lock().clone()
    }

    /// 把一份已有的来源授权接到这个 Pole 上：根 Grant 接手，一处权威。
    pub(crate) fn attach_source(&self, authorization: &Arc<SourceAuthorization>) {
        *self.authorization.lock() = Some(authorization.clone());
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

    pub(crate) fn reserve_records(&self, count: usize) -> Result<(), PieFail> {
        self.maps
            .lock()
            .try_reserve(count)
            .map_err(|_| PieFail::OoM)
    }

    /// 把一段已有关系关联到根 Grant：创建事务的 Input 交接。
    ///
    /// 它成为该 token 下的活跃关系——`open` 会复用它，撤销也关它。
    pub(crate) fn adopt_input(&self, token: PieToken, space: &Arc<Space>, view: Arc<View>) {
        self.add(view, space, Some(token), Relation::Input);
    }

    fn add(
        &self,
        view: Arc<View>,
        space: &Arc<Space>,
        token: Option<PieToken>,
        relation: Relation,
    ) {
        self.maps
            .lock()
            .push(HeldMap::placed(view, space, token, relation));
    }

    /// 把一段已落地的关系记入该 token：先核对授权记录，再入活跃 View 表。
    pub(crate) fn record(
        &self,
        token: PieToken,
        space: &Arc<Space>,
        view: Arc<View>,
        staged: Option<TeamId>,
        opened: bool,
    ) -> Result<(), Arc<View>> {
        let permitted = self
            .authorities
            .lock()
            .iter()
            .any(|record| record.authorizes(token) && record.permitted());
        if !self.alive() || !permitted {
            return Err(view);
        }
        self.maps.lock().push(HeldMap::placed(
            view,
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

    pub(crate) fn has_maps(&self) -> bool {
        !self.maps.lock().is_empty()
    }

    pub(crate) fn close_maps(&self, token: Option<PieToken>) -> Result<(), PieFail> {
        loop {
            let entry = {
                let mut maps = self.maps.lock();
                let index = maps
                    .iter()
                    .position(|entry| token.is_none_or(|token| entry.closable(token)));
                match index {
                    Some(index) => maps.remove(index),
                    None => return Ok(()),
                }
            };
            if let Some(space) = entry.placement.as_ref().and_then(Weak::upgrade) {
                let result = (|| {
                    space.nest.retry_invalidation()?;
                    if space.nest.contains_reservation(entry.view.identity()) {
                        space.withdraw(entry.view.whole())?;
                    }
                    Ok::<_, NestError>(())
                })();
                if let Err(error) = result {
                    self.maps.lock().push(entry);
                    return Err(pie_map_error(error));
                }
                if !space.nest.contains_reservation(entry.view.identity()) {
                    space.retire_view(entry.view.identity());
                }
            }
        }
    }

    pub(crate) fn withdraw(&self, token: PieToken) {
        loop {
            let entry = {
                let mut maps = self.maps.lock();
                match maps.iter().position(|entry| entry.closable(token)) {
                    Some(index) => maps.remove(index),
                    None => break,
                }
            };
            if let Some(weak) = entry.placement {
                if let Some(space) = weak.upgrade() {
                    let id = entry.view.identity();
                    let _ = space.dispose(entry.view);
                    if !space.nest.contains_reservation(id) {
                        space.retire_view(id);
                    }
                }
            }
        }
    }
}

/// 这条关系能不能代表**整份来源**被复用。
///
/// 来源窗口必须与 Pole 整窗一致（同一 Flat 的同一段），活动区间必须从起点连续
/// 覆盖到末尾：Unseal 前允许同源多个 Attach（slice、中间有洞），那些关系仍归撤销
/// 清理，但不能顶替整窗去复用——否则会把 slice 的第 1 页当成源 0 页，或报整长
/// 却只有几个 PTE。
fn covers_source(meta: &PoleMeta, entry: &HeldMap) -> bool {
    let view = &entry.view;
    if !view.source().same_range(&meta.source) {
        return false;
    }
    let (start, size) = view.initial_range();
    let Ok(active) = view.active() else {
        return false;
    };
    let mut covered = 0usize;
    for range in active {
        if range.start.as_usize() != start.as_usize() + covered {
            return false;
        }
        covered += range.size.get();
    }
    covered == size
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
                    let id = entry.view.identity();
                    let _ = space.dispose(entry.view);
                    if !space.nest.contains_reservation(id) {
                        space.retire_view(id);
                    }
                }
            }
        }
    }
}

/// 把一条关系的每个活动区间的上限与当前权限都收到 `access`。
///
/// 收窄是幂等的：已在同一上限上的区间重放不改变库存与 PTE。
fn narrow_view(space: &Arc<Space>, view: &Arc<View>, access: Access) -> Result<(), NestError> {
    for range in view.active()? {
        let offset = (range.start.as_usize() - view.initial_start().as_usize()) / PAGE_SIZE;
        let count = PageCount::new(range.size.get() / PAGE_SIZE).ok_or(NestError::NotAligned)?;
        space.narrow(view.range(PageOffset::new(offset), count)?, access)?;
    }
    Ok(())
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
                entry.holds(token)
                    && entry.opened()
                    && entry.placed_in(space)
                    && entry.live()
                    && covers_source(meta, entry)
            })
            .map(|entry| entry.view.clone())
    };
    if let Some(view) = reusable {
        narrow_view(space, &view, access).map_err(pie_map_error)?;
        return Ok((view.initial_range().0.as_usize(), meta.size()));
    }
    meta.reserve_records(1)?;
    // 来源已在手上：先选址登记，再按来源范围挂载，不经任何 kernel / canonical 视图。
    let source = meta.source();
    let size = source
        .page_count()
        .checked_bytes()
        .ok_or(PieFail::NotAligned)?;
    let reservation = space
        .nest
        .reserve(Placement::lowest(RegionId::Normal, size, PAGE_SIZE).map_err(pie_map_error)?)
        .map_err(pie_map_error)?;
    let view = space
        .nest
        .attach(reservation, source, access, access)
        .map_err(pie_map_error)?;
    let address = view.initial_start().as_usize();
    if !meta.alive() || grant.permission().is_none() {
        let _ = space.dispose(view);
        return Err(PieFail::Dead);
    }
    if let Err(view) = meta.record(token, space, view, None, true) {
        let _ = space.dispose(view);
        return Err(PieFail::Dead);
    }
    Ok((address, meta.size()))
}

pub(crate) fn shut(meta: &Arc<PoleMeta>, token: PieToken) -> Result<(), PieFail> {
    let _operation = PoleOperation::acquire(meta).ok_or(PieFail::Busy)?;
    meta.close_maps(Some(token))
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
            // 属主已关闭的关系与旧路径一样当场跳过，而不是把关闭折成拒绝。
            if entry.view.owner().is_none() {
                continue;
            }
            if let Some(space) = entry.placement.as_ref().and_then(Weak::upgrade) {
                targets.push((space, entry.view.clone()));
            }
        }
    }
    for (space, view) in targets {
        narrow_view(&space, &view, access).map_err(pie_map_error)?;
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
