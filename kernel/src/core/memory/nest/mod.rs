//! CPU 翻译：地址占用、翻译关系、页表、身份与同步退休。
//!
//! Flat 继续独立管理来源与物化；这里不保存来源页槽。

use alloc::boxed::Box;
use alloc::sync::Arc;
use alloc::vec::Vec;
use core::mem::ManuallyDrop;
use core::sync::atomic::{AtomicBool, Ordering};

use riscv::register::satp;

use crate::core::memory::PAGE_SIZE;
use crate::core::memory::address::{PageCount, PageOffset, PhysAddr, VirtAddr};
use crate::core::memory::flat::{Frame, Pages};
use crate::machine::layout::{TEAM_FRAME_BASE, TEAM_FRAME_WINDOW_SIZE, TRAP_STACK_BASE};
use crate::machine::mode;
use crate::machine::sync::{Level, SpinLock, SpinLockGuard};

mod address;
mod cleanup;
mod error;
mod fault;
mod index;
mod kind;
mod reservation;
pub(crate) mod source_views;
mod state;
mod unpublish;

pub(crate) mod identity;
pub(crate) mod synchronize;
pub(crate) mod table;
pub(crate) mod view;

#[cfg(debug_assertions)]
pub(crate) use unpublish::fail_unpublish_prepare;

pub use error::NestError;
pub use fault::FaultKind;
pub use identity::Asid;
pub use kind::SpaceKind;
pub use reservation::Reservation;
pub use view::{Access, ActiveRange, View};

pub(crate) use address::{Placement, PlacementAt, RegionId};
pub(crate) use view::ViewRange;

use address::Occupancy;
use cleanup::CleanupNode;
use index::IndexNode;
use state::NestState;
use synchronize::Invalidation;
use table::{PteFlags, TableNode};

#[cfg(debug_assertions)]
static FAIL_CLEANUP: AtomicBool = AtomicBool::new(false);

#[cfg(debug_assertions)]
static FAIL_WITHDRAW_PREPARE: AtomicBool = AtomicBool::new(false);

#[cfg(debug_assertions)]
static FAIL_PUBLISH_PREPARE: AtomicBool = AtomicBool::new(false);

#[cfg(debug_assertions)]
pub(crate) fn fail_cleanup_prepare() {
    assert!(crate::testing());
    FAIL_CLEANUP.store(true, Ordering::Relaxed);
}

#[cfg(debug_assertions)]
pub(crate) fn fail_withdraw_prepare() {
    assert!(crate::testing());
    FAIL_WITHDRAW_PREPARE.store(true, Ordering::Relaxed);
}

/// 下一次安装时让“发布记录容量”那一批准备失败：它必须在任何 PTE 写入之前。
#[cfg(debug_assertions)]
pub(crate) fn fail_publish_prepare() {
    assert!(crate::testing());
    FAIL_PUBLISH_PREPARE.store(true, Ordering::Relaxed);
}

/// 一个 CPU 地址空间：身份、根页表、全局占用、关系与同步退休。
///
/// 关系（`View`）自己持有来源与活动库存；这里只保存它们的索引、
/// 待失效标记和退休中的资源。占用表只登记跨度，不物化、不发布。
pub struct Nest {
    asid: Asid,
    kind: SpaceKind,
    geometry: mode::Geo,
    occupancy: SpinLock<Occupancy>,
    pending_cleanup: SpinLock<Option<Box<CleanupNode>>>,
    state: ManuallyDrop<SpinLock<NestState>>,
    asid_released: AtomicBool,
    published: AtomicBool,
}

impl Nest {
    /// 用户地址空间：动态取一个 ASID，窗口按平台布局登记。
    pub(crate) fn new(kind: SpaceKind) -> Result<Arc<Self>, NestError> {
        Self::build(kind, false)
    }

    /// 内核地址空间：ASID 0 固定，`Resource` 区可用，生命周期为进程期。
    pub(crate) fn kernel(kind: SpaceKind) -> Result<Arc<Self>, NestError> {
        Self::build(kind, true)
    }

    fn build(kind: SpaceKind, kernel: bool) -> Result<Arc<Self>, NestError> {
        let root = TableNode::root()?;
        let mut occupancy = Occupancy::new();
        occupancy.attach(
            RegionId::Normal,
            crate::machine::layout::IMAGE_BASE.as_usize(),
            mode::upper().as_usize() - 1,
        )?;
        occupancy.attach(
            RegionId::Kernel,
            TEAM_FRAME_BASE.as_usize(),
            TEAM_FRAME_BASE.as_usize() + TEAM_FRAME_WINDOW_SIZE - 1,
        )?;
        if kernel {
            let dram = crate::machine::board::info().dram;
            let base = mode::lower()
                .as_usize()
                .checked_add(dram.base)
                .and_then(|base| base.checked_add(dram.size))
                .ok_or(NestError::DramOverlap)?;
            occupancy.attach(RegionId::Resource, base, TRAP_STACK_BASE.as_usize() - 1)?;
        }
        let asid = if kernel {
            Asid::kernel()
        } else {
            Asid::try_allocate().ok_or(NestError::OutOfMemory)?
        };
        let nest = Arc::try_new(Self {
            asid,
            kind,
            geometry: mode::geometry(mode::mode()),
            occupancy: SpinLock::new_level(Level::Space, occupancy),
            pending_cleanup: SpinLock::new_level(Level::Space, None),
            state: ManuallyDrop::new(SpinLock::new_level(Level::Nest, NestState::new(root))),
            asid_released: AtomicBool::new(kernel),
            published: AtomicBool::new(false),
        });
        match nest {
            Ok(mut nest) => {
                Arc::get_mut(&mut nest)
                    .expect("new Arc is unique")
                    .published
                    .store(true, Ordering::Release);
                Ok(nest)
            }
            Err(_) => {
                if !asid.is_kernel() {
                    Asid::release_unpublished(asid);
                }
                Err(NestError::OutOfMemory)
            }
        }
    }

    /// 尚未发布的事务只在这些阶段可见：Clean 且未关闭。
    fn ready_state(&self) -> Result<SpinLockGuard<'_, NestState>, NestError> {
        loop {
            let state = self.state.lock();
            if state.phase.is_closed() {
                return Err(NestError::NoRegion);
            }
            match state.invalidation {
                Invalidation::Clean => return Ok(state),
                Invalidation::Failed => return Err(NestError::Shootdown),
                Invalidation::Dirty | Invalidation::Fencing => {}
            }
            drop(state);
            self.finish_invalidation()?;
        }
    }

    pub fn kind(&self) -> SpaceKind {
        self.kind
    }

    pub(crate) fn asid(&self) -> Asid {
        self.asid
    }

    /// 页表几何：健康面核对加载模式换算出的层数。
    #[cfg(debug_assertions)]
    pub(crate) fn geometry(&self) -> mode::Geo {
        self.geometry
    }

    pub(crate) fn root_ppn(&self) -> usize {
        self.state.lock().root.ppn()
    }

    pub(crate) fn satp(&self) -> satp::Satp {
        satp::Satp::from_bits(
            (mode::mode().into_usize() << 60) | (self.asid.get() << 44) | self.root_ppn(),
        )
    }

    #[cfg(debug_assertions)]
    pub(crate) fn view_count(&self) -> usize {
        self.state.lock().index.len()
    }

    #[cfg(debug_assertions)]
    pub(crate) fn table_count(&self) -> usize {
        let state = self.state.lock();
        state.root.node_count()
            + state
                .retired_tables
                .iter()
                .map(TableNode::node_count)
                .sum::<usize>()
    }

    pub(crate) fn probe(&self, address: VirtAddr) -> Option<Arc<View>> {
        self.ready_state().ok()?.index.find(address.as_usize())
    }

    /// 是否有活关系与 `[start, start + size)` 相交。
    pub(crate) fn overlaps(&self, start: VirtAddr, size: usize) -> Result<bool, NestError> {
        if size == 0 || start.offset() != 0 || !size.is_multiple_of(PAGE_SIZE) {
            return Err(NestError::NotAligned);
        }
        let end = start
            .as_usize()
            .checked_add(size - 1)
            .ok_or(NestError::NotAligned)?;
        let state = self.state.lock();
        Ok(state.index.first_overlap(start.as_usize(), end).is_some())
    }

    pub(crate) fn has_pending_invalidation(&self) -> bool {
        !self.state.lock().invalidation.is_clean()
    }

    pub(crate) fn contains_reservation(&self, id: usize) -> bool {
        self.occupancy.lock().has_id(id)
    }

    #[cfg(debug_assertions)]
    pub(crate) fn pending_views(&self) -> Result<Vec<Arc<View>>, NestError> {
        let state = self.state.lock();
        let mut views = Vec::new();
        views
            .try_reserve(state.pending_views.len())
            .map_err(|_| NestError::OutOfMemory)?;
        views.extend(state.pending_views.iter().cloned());
        Ok(views)
    }

    /// 全部活动区间及其所属关系，按区间起点升序。
    pub(crate) fn active_ranges(&self) -> Result<Vec<(Arc<View>, ActiveRange)>, NestError> {
        let state = self.state.lock();
        let mut views = Vec::new();
        views
            .try_reserve(state.index.len())
            .map_err(|_| NestError::OutOfMemory)?;
        state.index.collect(&mut views);
        let count = views
            .iter()
            .map(|view| view.state.lock().active.len())
            .sum();
        let mut ranges = Vec::new();
        ranges
            .try_reserve(count)
            .map_err(|_| NestError::OutOfMemory)?;
        for view in views {
            ranges.extend(
                view.state
                    .lock()
                    .active
                    .iter()
                    .copied()
                    .map(|range| (view.clone(), range)),
            );
        }
        ranges.sort_unstable_by_key(|(_, range)| range.start);
        Ok(ranges)
    }

    /// 普通挂载的架构属性：用户空间只对 `Normal` 区开 U，A/D 由内核自维护。
    pub(crate) fn attrs(&self, region: RegionId) -> PteFlags {
        let mut attrs = PteFlags::A | PteFlags::D;
        if !self.kind.is_supervisor() && region == RegionId::Normal {
            attrs |= PteFlags::U;
        }
        attrs
    }

    /// 重设窗口边界；`edge` 是窗口的排他末端。
    #[cfg(debug_assertions)]
    pub(crate) fn attach_region(
        &self,
        region: RegionId,
        base: usize,
        edge: usize,
    ) -> Result<(), NestError> {
        self.occupancy.lock().configure(region, base, edge - 1)
    }

    /// 抬高窗口的选址下界；已登记的占用不动。
    pub(crate) fn advance_region(&self, region: RegionId, base: usize) -> Result<(), NestError> {
        self.occupancy.lock().advance(region, base)
    }

    /// 该区域的动态窗口是否含住 `[start, start + size)`。
    pub(crate) fn window_contains(&self, region: RegionId, start: VirtAddr, size: usize) -> bool {
        let Some(last) = start.as_usize().checked_add(size - 1) else {
            return false;
        };
        self.occupancy
            .lock()
            .contains(region, start.as_usize(), last)
    }

    /// 选一段跨度并登记占用；不动页表、不物化来源。
    pub(crate) fn reserve(
        self: &Arc<Self>,
        placement: Placement,
    ) -> Result<Reservation, NestError> {
        let (start, id) = self.occupancy.lock().reserve(placement)?;
        Ok(Reservation::new(
            Arc::downgrade(self),
            placement.region(),
            VirtAddr::from_raw(start),
            placement.size(),
            id,
        ))
    }

    /// 普通挂载：整段初始范围一条活动记录，属性按种类与区域政策算出。
    pub(crate) fn attach(
        self: &Arc<Self>,
        reservation: Reservation,
        source: crate::core::memory::flat::FlatRange,
        ceiling: Access,
        current: Access,
    ) -> Result<Arc<View>, NestError> {
        if let Err(error) = self.check_owner(&reservation) {
            return Err(Self::abandon(reservation, error));
        }
        let attrs = self.attrs(reservation.region());
        let active =
            match ActiveRange::new(reservation.start(), reservation.size(), ceiling, current) {
                Ok(active) => [active],
                Err(error) => return Err(Self::abandon(reservation, error)),
            };
        self.attach_shaped(reservation, source, attrs, &active)
    }

    /// 按给定活动形状挂载：与普通入口共用一个私有安装/提交。
    ///
    /// 只接真实占用、真实来源与明确的活动区间；不接来源关系、
    /// 不接第二份可伪造的地址投影。占用必须是本 Nest 的（否则 `NoRegion`），
    /// 未发布的失败退回它的属主，已发布的失败进清理链。
    pub(crate) fn attach_shaped(
        self: &Arc<Self>,
        reservation: Reservation,
        source: crate::core::memory::flat::FlatRange,
        attrs: PteFlags,
        active: &[ActiveRange],
    ) -> Result<Arc<View>, NestError> {
        if let Err(error) = self.check_owner(&reservation) {
            return Err(Self::abandon(reservation, error));
        }
        if !View::attrs_are_architectural(attrs) {
            return Err(Self::abandon(reservation, NestError::NotAligned));
        }
        let mut ranges = Vec::new();
        if ranges.try_reserve(active.len()).is_err() {
            return Err(Self::abandon(reservation, NestError::OutOfMemory));
        }
        ranges.extend_from_slice(active);
        let view = View::new(reservation, attrs, ranges, source)?;
        match self.commit(&view) {
            Ok(()) => Ok(view),
            Err(error) => {
                self.settle_failed_install(&view);
                Err(error)
            }
        }
    }

    /// 一笔尚未发布的占用的归还方向：它的属主自己的表，不是接收调用的那一方。
    ///
    /// 占用身份是全局唯一的，所以“拿裸坐标去目标表里找一笔”不会误删别人的槽；
    /// 但那样也退不掉真属主的这一笔，因此归还一律按属主。
    fn give_back_unpublished(reservation: &Reservation) -> bool {
        reservation
            .nest()
            .is_some_and(|owner| owner.give_back(reservation))
    }

    /// 占用已经交出、本笔操作不再有承接者时唯一的归还路径：按属主交还并交出错误。
    ///
    /// 与调用方主动退的 `release_reservation` 成对；`give_back` / `give_back_span`
    /// 只是“退还”的原语，不包含“这笔操作失败”的语义。
    fn abandon(reservation: Reservation, error: NestError) -> NestError {
        Self::give_back_unpublished(&reservation);
        error
    }

    /// 安装失败的责任划分：已发布就把回收交给清理链（占用仍禁复用），
    /// 未发布就退回占用与临时数据。
    fn settle_failed_install(&self, view: &Arc<View>) {
        let published = {
            let state = self.state.lock();
            state
                .index
                .find(view.initial_start().as_usize())
                .is_some_and(|held| Arc::ptr_eq(&held, view))
        };
        if !published {
            Self::give_back_unpublished(&view.reservation);
            return;
        }
        // 取节点要先把占用锁放掉：它与清理链同属 `Level::Space`，套着取就是同级递归。
        let node = self.occupancy.lock().take_cleanup(view.reservation.id());
        if let Some(mut node) = node {
            node.view = Some(view.clone());
            self.enqueue_cleanup(node);
        }
    }

    /// 安装/提交：所有可失败的元数据、几何与清理载荷都先备齐，
    /// 之后的 PTE 写入与索引接入不再失败。
    fn commit(self: &Arc<Self>, view: &Arc<View>) -> Result<(), NestError> {
        let reservation = &view.reservation;
        let start = reservation.start();
        let end = reservation.last();
        {
            let mut occupancy = self.occupancy.lock();
            if !occupancy.has_id(reservation.id()) {
                return Err(NestError::NoRegion);
            }
            #[cfg(debug_assertions)]
            if FAIL_CLEANUP.swap(false, Ordering::Relaxed) {
                return Err(NestError::OutOfMemory);
            }
            occupancy.prepare_cleanup(reservation.id())?;
        }
        // 本关系要发布哪些页：Managed 只取已物化的，缺口留给缺页；
        // Borrowed 的整段都可用，挂载时就全部发布。
        let mut published: Vec<(usize, Arc<Frame>)> = Vec::new();
        let mut borrowed: Vec<(VirtAddr, usize, Access)> = Vec::new();
        let mut pages: Option<Pages> = None;
        // 发布窗口的短期驻留：在抓承载快照**之前**取得，覆盖 PTE 写入与发布记录提交，
        // 之后才解除。在途的移交槽会让这次驻留当场 Busy，正好与事务互斥。
        let lease = {
            let inner = view.state.lock();
            inner.source.reside()?
        };
        {
            let inner = view.state.lock();
            if inner.source.is_borrowed() {
                pages = Some(inner.source.materialize()?);
                borrowed
                    .try_reserve(inner.active.len())
                    .map_err(|_| NestError::OutOfMemory)?;
                for range in &inner.active {
                    borrowed.push((range.start, range.size.get(), range.current));
                }
            } else {
                let materialized = inner.source.materialized_frames()?;
                published
                    .try_reserve(materialized.len())
                    .map_err(|_| NestError::OutOfMemory)?;
                for (offset, frame) in materialized {
                    let page = offset.get();
                    let address = start + page * PAGE_SIZE;
                    if inner.covers(address.as_usize(), address.as_usize()) {
                        published.push((page, frame));
                    }
                }
            }
        }
        // 发布记录容量是安装前的准备：与 PTE 写入同一批，备不齐就不进入安装。
        // 于是“失败时还没有任何 PTE”对 `settle_failed_install` 的判据成立。
        {
            let mut inner = view.state.lock();
            #[cfg(debug_assertions)]
            if FAIL_PUBLISH_PREPARE.swap(false, Ordering::Relaxed) {
                return Err(NestError::OutOfMemory);
            }
            inner
                .published
                .try_reserve(published.len())
                .map_err(|_| NestError::OutOfMemory)?;
        }
        let node =
            Box::try_new(IndexNode::new(view.clone())?).map_err(|_| NestError::OutOfMemory)?;
        let mut state = self.ready_state()?;
        if state.index.first_overlap(start.as_usize(), end).is_some() {
            return Err(NestError::AlreadyMapped);
        }
        state
            .pending_views
            .try_reserve(1)
            .map_err(|_| NestError::OutOfMemory)?;
        // 首条 PTE 之前登记来源↔关系：撤页要按来源枚举全部关系，不能只看用户注册表。
        source_views::register(&view.state.lock().source, view)?;
        for run in view.state.lock().runs(start, &published) {
            let run = run?;
            state
                .root
                .prepare_range_geometry(run.start, run.physical, run.size, self.geometry)?;
        }
        for (at, len, _) in &borrowed {
            let pa = pages
                .as_ref()
                .expect("borrowed runs imply a borrowed source")
                .physical(PageOffset::new(
                    (at.as_usize() - start.as_usize()) / PAGE_SIZE,
                ))?;
            state
                .root
                .prepare_range_geometry(*at, pa, *len, self.geometry)?;
        }
        // 可失败准备到此为止：这条关系随后只进不可失败的 PTE 提交，
        // 而短期驻留仍守着这份来源。内核地址空间与全局条目都让整份来源
        // 永久失去迁移资格。
        if self.asid.is_kernel() || view.is_global() {
            view.state.lock().source.mark_untransferable();
        }
        state.pending_views.push(view.clone());
        for run in view.state.lock().runs(start, &published) {
            let run = run?;
            state.root.map_range_prepared_geometry(
                run.start,
                run.physical,
                run.size,
                view.leaf_flags(run.current),
                self.geometry,
            );
        }
        for (at, len, current) in &borrowed {
            let pa = pages
                .as_ref()
                .expect("borrowed runs imply a borrowed source")
                .physical(PageOffset::new(
                    (at.as_usize() - start.as_usize()) / PAGE_SIZE,
                ))?;
            state.root.map_range_prepared_geometry(
                *at,
                pa,
                *len,
                view.leaf_flags(*current),
                self.geometry,
            );
        }
        {
            let mut inner = view.state.lock();
            for (page, frame) in published {
                inner.publish(page, frame);
            }
        }
        state.index.insert(node);
        state.mark_dirty();
        drop(state);
        drop(lease);
        self.finish_invalidation()
    }

    /// 把该窗口里已经物化/借用的页发布到本关系：不动 Flat 的物化。
    ///
    /// 只发布活动形状内的页，不偷分配未物化的页。
    fn publish(self: &Arc<Self>, selection: ViewRange) -> Result<(), NestError> {
        let view = selection.view().clone();
        for (start, size) in self.active_spans(&selection)? {
            self.publish_span(&view, start, size)?;
        }
        Ok(())
    }

    fn publish_span(
        self: &Arc<Self>,
        view: &Arc<View>,
        start: VirtAddr,
        size: usize,
    ) -> Result<(), NestError> {
        if size == 0 || start.offset() != 0 || !size.is_multiple_of(PAGE_SIZE) {
            return Err(NestError::NotAligned);
        }
        let end = start
            .as_usize()
            .checked_add(size - 1)
            .ok_or(NestError::NotAligned)?;
        let mut state = self.ready_state()?;
        self.ensure_owned(view, &state)?;
        let mut inner = view.state.lock();
        if !inner.covers(start.as_usize(), end) {
            return Err(NestError::NoRegion);
        }
        if inner.source.is_borrowed() {
            return Ok(());
        }
        let first = view.page_index(start);
        let window = inner.source.range(
            PageOffset::new(first),
            PageCount::new(size / PAGE_SIZE).ok_or(NestError::NotAligned)?,
        )?;
        // 发布窗口的短期驻留：在抓承载快照**之前**取得，覆盖 PTE 写入与发布记录提交。
        // 在途的移交槽会让这次驻留当场 Busy，正好与事务互斥。
        let lease = window.reside()?;
        let materialized = window.materialized_frames()?;
        let mut fresh = Vec::new();
        fresh
            .try_reserve(materialized.len())
            .map_err(|_| NestError::OutOfMemory)?;
        for (offset, frame) in materialized {
            let page = first + offset.get();
            if !inner.publishes(page) {
                fresh.push((page, frame));
            }
        }
        if fresh.is_empty() {
            return Ok(());
        }
        inner
            .published
            .try_reserve(fresh.len())
            .map_err(|_| NestError::OutOfMemory)?;
        state
            .pending_views
            .try_reserve(1)
            .map_err(|_| NestError::OutOfMemory)?;
        for run in inner.runs(view.initial_start(), &fresh) {
            let run = run?;
            state
                .root
                .prepare_range_geometry(run.start, run.physical, run.size, self.geometry)?;
        }
        // 可失败准备到此为止：随后只有这一页的 PTE 提交，短期驻留仍守着这份来源。
        // 内核地址空间与全局条目都让整份来源永久失去迁移资格。
        if self.asid.is_kernel() || view.is_global() {
            inner.source.mark_untransferable();
        }
        state.pending_views.push(view.clone());
        for run in inner.runs(view.initial_start(), &fresh) {
            let run = run?;
            state.root.map_range_prepared_geometry(
                run.start,
                run.physical,
                run.size,
                view.leaf_flags(run.current),
                self.geometry,
            );
        }
        for (page, frame) in fresh {
            inner.publish(page, frame);
        }
        state.mark_dirty();
        drop(inner);
        drop(state);
        drop(lease);
        self.finish_invalidation()
    }

    /// 让 `Flat` 物化该窗口的活动页，再在本关系发布。
    ///
    /// 组合入口：物化原语仍是 `FlatRange::materialize`，这里只把缺页与派生编排
    /// 要的两步（先 Flat、再本关系 publish）接在一起，不是第二套物化权威。
    pub(crate) fn materialize_active(
        self: &Arc<Self>,
        selection: ViewRange,
    ) -> Result<(), NestError> {
        let view = selection.view().clone();
        for (start, size) in self.active_spans(&selection)? {
            let offset = PageOffset::new(view.page_index(start));
            let count = PageCount::new(size / PAGE_SIZE).ok_or(NestError::NotAligned)?;
            let part = view.range(offset, count)?;
            part.source()?.materialize()?;
            self.publish(part)?;
        }
        Ok(())
    }

    /// 发布只选择活动页，不把尚未结算的撤销重新安装。
    fn active_spans(&self, selection: &ViewRange) -> Result<Vec<(VirtAddr, usize)>, NestError> {
        let view = selection.view();
        let (lo, hi) = selection.window();
        let state = self.ready_state()?;
        self.ensure_owned(view, &state)?;
        let inner = view.state.lock();
        if inner.active.is_empty()
            || (matches!(selection, ViewRange::Pages { .. }) && !inner.covers(lo, hi))
        {
            return Err(NestError::NoRegion);
        }
        let mut spans = Vec::new();
        spans
            .try_reserve(inner.active.len())
            .map_err(|_| NestError::OutOfMemory)?;
        for range in &inner.active {
            if let Some((start, last)) = range.intersection(lo, hi) {
                spans.push((VirtAddr::from_raw(start), last - start + 1));
            }
        }
        Ok(spans)
    }

    /// 在权限上限内改选择窗口的当前逻辑权限；不改架构属性。
    pub(crate) fn set_access(&self, selection: ViewRange, access: Access) -> Result<(), NestError> {
        if !access.is_leaf_set() {
            return Err(NestError::NotAligned);
        }
        let view = selection.view().clone();
        let (lo, hi) = selection.window();
        let mut state = self.ready_state()?;
        self.ensure_owned(&view, &state)?;
        let mut inner = view.state.lock();
        if !inner.covers(lo, hi) {
            return Err(NestError::NoRegion);
        }
        if inner
            .active
            .iter()
            .filter(|range| range.overlaps(lo, hi))
            .any(|range| !range.ceiling.contains(access))
        {
            return Err(NestError::WidenDenied);
        }
        let next = ActiveRange::split(&inner.active, lo, hi, Some(access))?;
        let first = view.page_index(VirtAddr::from_raw(lo));
        let pages = (hi - lo + 1) / PAGE_SIZE;
        let borrowed = inner.source.is_borrowed();
        let mut updates = Vec::new();
        updates
            .try_reserve(pages)
            .map_err(|_| NestError::OutOfMemory)?;
        for page in first..first + pages {
            let va = view.initial_start() + page * PAGE_SIZE;
            if borrowed || inner.publishes(page) {
                state.root.prepare_geometry(va, PAGE_SIZE, self.geometry)?;
                state.root.walk_ref_geometry(va, self.geometry)?;
                updates.push(va);
            }
        }
        state
            .pending_views
            .try_reserve(1)
            .map_err(|_| NestError::OutOfMemory)?;
        state.pending_views.push(view.clone());
        for va in updates {
            state.root.set_permissions_prepared_geometry(
                va,
                view.leaf_flags(access),
                self.geometry,
            );
        }
        inner.active = next;
        drop(inner);
        state.index.refresh(&view);
        state.mark_dirty();
        drop(state);
        self.finish_invalidation()
    }

    /// 收窄选择窗口的权限上限与当前权限；架构属性不变。
    pub(crate) fn narrow(&self, selection: ViewRange, ceiling: Access) -> Result<(), NestError> {
        let view = selection.view().clone();
        let (lo, hi) = selection.window();
        let mut state = self.ready_state()?;
        self.ensure_owned(&view, &state)?;
        let mut inner = view.state.lock();
        if !inner.covers(lo, hi) {
            return Err(NestError::NoRegion);
        }
        let next = ActiveRange::narrow(&inner.active, lo, hi, ceiling)?;
        let first = view.page_index(VirtAddr::from_raw(lo));
        let pages = (hi - lo + 1) / PAGE_SIZE;
        let borrowed = inner.source.is_borrowed();
        let mut updates = Vec::new();
        updates
            .try_reserve(pages)
            .map_err(|_| NestError::OutOfMemory)?;
        for page in first..first + pages {
            let va = view.initial_start() + page * PAGE_SIZE;
            let Some(range) = next.iter().find(|range| range.contains(va.as_usize())) else {
                return Err(NestError::NoRegion);
            };
            if borrowed || inner.publishes(page) {
                state.root.prepare_geometry(va, PAGE_SIZE, self.geometry)?;
                state.root.walk_ref_geometry(va, self.geometry)?;
                updates.push((va, range.current));
            }
        }
        state
            .pending_views
            .try_reserve(1)
            .map_err(|_| NestError::OutOfMemory)?;
        state.pending_views.push(view.clone());
        for (va, current) in updates {
            state.root.set_permissions_prepared_geometry(
                va,
                view.leaf_flags(current),
                self.geometry,
            );
        }
        inner.active = next;
        drop(inner);
        state.index.refresh(&view);
        state.mark_dirty();
        drop(state);
        self.finish_invalidation()
    }

    /// 按选择撤下：先退休该段占用，再撤 PTE，同步成功后只回收这一段。
    ///
    /// 失败时退休票、Frame、表页、来源与待撤的活动全都保留，VA 禁复用；
    /// 沿用同一身份重试（再次 `detach`，或 `retry_invalidation`）才继续。
    pub(crate) fn detach(self: &Arc<Self>, selection: ViewRange) -> Result<(), NestError> {
        let view = selection.view().clone();
        self.check_owner(&view.reservation)?;
        if self.is_closed() {
            return Err(NestError::NoRegion);
        }
        if self.has_pending_invalidation() {
            // 先把上一次失败的同步结算掉，再动这一段。
            self.finish_invalidation()?;
        }
        for (start, len) in self.owed_spans(&selection)? {
            self.withdraw(&view, start, len)?;
        }
        Ok(())
    }

    /// 选择在当前状态下还欠着的段（活动 ∪ 未结算退休），按地址升序。
    ///
    /// `Pages` 还要求整个窗口都被欠着：选到已经结算完的页就是失败。
    /// 整段只取还欠着的那几段，因此部分撤下之后不会去碰已经没了的洞。
    fn owed_spans(&self, selection: &ViewRange) -> Result<Vec<(VirtAddr, usize)>, NestError> {
        let view = selection.view();
        let (lo, hi) = selection.window();
        let strict = matches!(selection, ViewRange::Pages { .. });
        let mut owned: Vec<(usize, usize)> = Vec::new();
        {
            let inner = view.state.lock();
            owned
                .try_reserve(inner.active.len() + 4)
                .map_err(|_| NestError::OutOfMemory)?;
            for range in &inner.active {
                if let Some((at, last)) = range.intersection(lo, hi) {
                    owned.push((at, last));
                }
            }
        }
        {
            let occupancy = self.occupancy.lock();
            for (start, last, _) in occupancy.retired_of(view.reservation.id())? {
                let at = start.max(lo);
                let last = last.min(hi);
                if at <= last {
                    owned.push((at, last));
                }
            }
        }
        owned.sort_unstable_by_key(|(start, _)| *start);
        let mut spans = Vec::new();
        spans
            .try_reserve(owned.len())
            .map_err(|_| NestError::OutOfMemory)?;
        let mut floor = lo;
        let mut covered = false;
        for (start, last) in owned {
            if last < floor {
                continue;
            }
            if start > floor {
                if strict {
                    return Err(NestError::NoRegion);
                }
                floor = start;
            }
            spans.push((VirtAddr::from_raw(floor), last - floor + 1));
            if last >= hi {
                covered = true;
                break;
            }
            floor = last + 1;
        }
        if strict && !covered {
            return Err(NestError::NoRegion);
        }
        Ok(spans)
    }

    /// 撤下一段：退休该段占用 → 撤 PTE → 同步 → 只回收这一段。
    fn withdraw(
        self: &Arc<Self>,
        view: &Arc<View>,
        start: VirtAddr,
        len: usize,
    ) -> Result<(), NestError> {
        if len == 0 || start.offset() != 0 || !len.is_multiple_of(PAGE_SIZE) {
            return Err(NestError::NotAligned);
        }
        let last = start
            .as_usize()
            .checked_add(len - 1)
            .ok_or(NestError::NotAligned)?;
        drop(self.ready_state()?);

        // 地址 → Nest → View：准备与提交共享临界区，不暴露无人接管的退休票。
        let mut occupancy = self.occupancy.lock();
        let pieces = occupancy.pieces(start.as_usize(), last, view.reservation.id())?;
        let fresh_count = pieces.iter().filter(|(_, _, ticket)| *ticket == 0).count();
        occupancy.reserve_cuts(fresh_count.checked_mul(2).ok_or(NestError::OutOfMemory)?)?;
        let mut tickets: Vec<usize> = Vec::new();
        tickets
            .try_reserve(pieces.len())
            .map_err(|_| NestError::OutOfMemory)?;
        let mut state = self.state.lock();
        if state.phase.is_closed() {
            return Err(NestError::NoRegion);
        }
        if !state.invalidation.is_clean() {
            return Err(NestError::Shootdown);
        }
        self.ensure_owned(view, &state)?;
        let mut inner = view.state.lock();
        let next = ActiveRange::split(&inner.active, start.as_usize(), last, None)?;
        let first_page = view.page_index(start);
        let last_page = view.page_index(VirtAddr::from_raw(last));
        let mut removed = 0;
        for (page, _) in &inner.published {
            if *page >= first_page && *page <= last_page {
                removed += 1;
            }
        }
        let next_node = if next.is_empty() {
            None
        } else {
            Some(
                Box::try_new(IndexNode::with_ranges(view.clone(), &next)?)
                    .map_err(|_| NestError::OutOfMemory)?,
            )
        };
        state
            .retired_frames
            .try_reserve(removed)
            .map_err(|_| NestError::OutOfMemory)?;
        state
            .pending_views
            .try_reserve(1)
            .map_err(|_| NestError::OutOfMemory)?;
        state
            .retirement
            .try_reserve(pieces.len())
            .map_err(|_| NestError::OutOfMemory)?;
        {
            let NestState {
                root,
                retired_tables,
                ..
            } = &mut *state;
            root.prepare_geometry(start, len, self.geometry)?;
            root.reserve_retired_tables(retired_tables)?;
        }
        #[cfg(debug_assertions)]
        if FAIL_WITHDRAW_PREPARE.swap(false, Ordering::Relaxed) {
            return Err(NestError::OutOfMemory);
        }

        // 开票之后没有可失败准备：票、PTE 与关系在同一提交内转入退休责任。
        for (lo, hi, ticket) in pieces {
            tickets.push(if ticket == 0 {
                occupancy.retire(lo, hi, view.reservation.id())
            } else {
                ticket
            });
        }
        {
            let NestState {
                root,
                retired_tables,
                ..
            } = &mut *state;
            root.unmap_prepared_geometry(start, len, self.geometry);
            root.reclaim_empty_tables(retired_tables);
        }
        state.pending_views.push(view.clone());
        for index in (0..inner.published.len()).rev() {
            let page = inner.published[index].0;
            if page >= first_page && page <= last_page {
                let (_, frame) = inner.published.remove(index);
                state.retired_frames.push(frame);
            }
        }
        inner.active = next;
        drop(inner);
        state.index.remove(view);
        if let Some(node) = next_node {
            state.index.insert(node);
        }
        state.retirement.extend(tickets.iter().copied());
        state.mark_dirty();
        drop(state);
        drop(occupancy);
        // 三、同步成功之后才结算这一段的票；失败时票、Frame、表页与来源都留着。
        self.finish_invalidation()?;
        {
            let mut state = self.state.lock();
            state.retirement.retain(|ticket| !tickets.contains(ticket));
        }
        if !tickets.is_empty() {
            let mut occupancy = self.occupancy.lock();
            for ticket in tickets {
                occupancy.reclaim(ticket);
            }
        }
        Ok(())
    }

    /// 接管一笔已落地关系的回收：消费式入口。
    ///
    /// 失败时责任留在 Nest 自己那里：退休票、退休资源与待撤的活动都还在，
    /// 由 `retry_invalidation` 继续；调用者不再持有这笔关系的凭证。
    pub(crate) fn dispose(self: &Arc<Self>, view: Arc<View>) -> Result<(), NestError> {
        self.check_owner(&view.reservation)?;
        let node = self.occupancy.lock().take_cleanup(view.reservation.id());
        let Some(mut node) = node else {
            return self.detach(view.whole());
        };
        node.view = Some(view.clone());
        let result = self.detach(view.whole());
        if let Err(error) = result {
            self.enqueue_cleanup(node);
            return Err(error);
        }
        Ok(())
    }

    /// 把一段占用对半分成两笔；失败时归还原占用。
    pub(crate) fn split_reservation(
        self: &Arc<Self>,
        reservation: Reservation,
        left_size: usize,
    ) -> Result<(Reservation, Reservation), NestError> {
        self.check_reservation(&reservation)?;
        let split = self.occupancy.lock().split(
            reservation.start().as_usize(),
            reservation.last(),
            reservation.id(),
            left_size,
        );
        let (right_start, left_id, right_id) = match split {
            Ok(split) => split,
            Err(error) => {
                self.give_back(&reservation);
                return Err(error);
            }
        };
        let nest = Arc::downgrade(self);
        let region = reservation.region();
        let start = reservation.start();
        let size = reservation.size();
        Ok((
            Reservation::new(nest.clone(), region, start, left_size, left_id),
            Reservation::new(
                nest,
                region,
                VirtAddr::from_raw(right_start),
                size - left_size,
                right_id,
            ),
        ))
    }

    /// 交还一笔尚未落地的保留。
    pub(crate) fn release_reservation(&self, reservation: Reservation) -> Result<(), NestError> {
        self.check_reservation(&reservation)?;
        if self.overlaps(reservation.start(), reservation.size())? {
            return Err(NestError::AlreadyMapped);
        }
        if self.give_back(&reservation) {
            Ok(())
        } else {
            Err(NestError::NoRegion)
        }
    }

    fn give_back(&self, reservation: &Reservation) -> bool {
        self.give_back_span(reservation.start(), reservation.size(), reservation.id())
    }

    /// 按占用身份交还一段尚未落地的跨度（凭证已经不在手里时用）。
    fn give_back_span(&self, start: VirtAddr, size: usize, identity: usize) -> bool {
        self.occupancy
            .lock()
            .release(start.as_usize(), start.as_usize() + (size - 1), identity)
    }

    /// 结算待失效与退休，再收尾清理链上已经消费的关系。
    ///
    /// 这是显式的“原身份重试”入口：它把还欠着的占用票一并交还，然后把已消费的
    /// 关系收到手。正常关闭别的 View 不会走到这里，因此也不会错误结清失败的那段。
    pub(crate) fn retry_invalidation(self: &Arc<Self>) -> Result<(), NestError> {
        self.finish_invalidation()?;
        let tickets = {
            let mut state = self.state.lock();
            core::mem::take(&mut state.retirement)
        };
        if !tickets.is_empty() {
            let mut occupancy = self.occupancy.lock();
            for ticket in tickets {
                occupancy.reclaim(ticket);
            }
        }
        while let Some(node) = self.take_cleanup() {
            let Some(view) = node.view.clone() else {
                continue;
            };
            // 已消费的关系只剩收尾：PTE、退休票与退休资源都已清掉时这里是幂等的。
            if let Err(error) = self.detach(view.whole()) {
                self.enqueue_cleanup(node);
                return Err(error);
            }
        }
        Ok(())
    }

    fn finish_invalidation(&self) -> Result<(), NestError> {
        loop {
            let mut state = self.state.lock();
            if state.invalidation.is_clean() {
                return Ok(());
            }
            if state.invalidation.is_fencing() {
                drop(state);
                while self.state.lock().invalidation.is_fencing() {
                    core::hint::spin_loop();
                }
                continue;
            }
            state.fence();
            drop(state);

            let result = identity::shootdown(self.asid).map_err(|_| NestError::Shootdown);
            let retired_tables;
            let retired_frames;
            let pending_views;
            {
                let mut state = self.state.lock();
                if let Err(error) = result {
                    state.fail();
                    return Err(error);
                }
                state.settle();
                retired_tables = core::mem::take(&mut state.retired_tables);
                retired_frames = core::mem::take(&mut state.retired_frames);
                pending_views = core::mem::take(&mut state.pending_views);
            }
            // 退休 Frame / 表页可以随本次失效结清（shootdown 按 ASID 覆盖全部映射），
            // 但占用票不在这里回收：它只由该段的本次撤下或显式 `retry_invalidation` 结算。
            drop(retired_tables);
            drop(retired_frames);
            drop(pending_views);
            return Ok(());
        }
    }

    /// 撤下全部关系并等算 shootdown；内核地址空间为进程期，不在此列。
    pub(crate) fn close(&self) -> Result<(), NestError> {
        if self.asid.is_kernel() {
            return Err(NestError::Pinned);
        }
        let mut state = loop {
            let state = self.state.lock();
            if state.phase.is_closed() {
                return Ok(());
            }
            if state.invalidation.is_clean() {
                break state;
            }
            drop(state);
            self.finish_invalidation()?;
        };
        let mut views = Vec::new();
        views
            .try_reserve(state.index.len())
            .map_err(|_| NestError::OutOfMemory)?;
        state.index.collect(&mut views);
        let frame_count = views
            .iter()
            .map(|view| view.state.lock().published.len())
            .sum();
        state
            .retired_frames
            .try_reserve(frame_count)
            .map_err(|_| NestError::OutOfMemory)?;
        state
            .pending_views
            .try_reserve(views.len())
            .map_err(|_| NestError::OutOfMemory)?;
        {
            let NestState {
                root,
                retired_tables,
                ..
            } = &mut *state;
            for view in &views {
                for range in &view.state.lock().active {
                    root.prepare_geometry(range.start, range.size.get(), self.geometry)?;
                }
            }
            root.reserve_retired_tables(retired_tables)?;
            for view in &views {
                for range in &view.state.lock().active {
                    root.unmap_keep_tables_geometry(range.start, range.size.get(), self.geometry)?;
                }
            }
            root.reclaim_empty_tables(retired_tables);
        }
        for view in views {
            state.pending_views.push(view.clone());
            let mut inner = view.state.lock();
            for (_, frame) in inner.published.drain(..) {
                state.retired_frames.push(frame);
            }
            inner.active.clear();
        }
        state.index.clear();
        state.mark_dirty();
        state.begin_close();
        drop(state);
        self.finish_invalidation()
    }

    fn enqueue_cleanup(&self, mut node: Box<CleanupNode>) {
        let mut pending = self.pending_cleanup.lock();
        node.next = pending.take();
        *pending = Some(node);
    }

    /// 取一环待清理的消费式任务；处理完不再放回。
    fn take_cleanup(&self) -> Option<Box<CleanupNode>> {
        let mut pending = self.pending_cleanup.lock();
        let mut node = pending.take()?;
        *pending = node.next.take();
        Some(node)
    }

    /// 选择的所属与阶段：认住它的那个 Nest，且没有关闭。
    fn ensure_owned(&self, view: &Arc<View>, state: &NestState) -> Result<(), NestError> {
        let owner = view.reservation.nest().ok_or(NestError::NoRegion)?;
        if state.phase.is_closed()
            || !core::ptr::eq(Arc::as_ptr(&owner) as *const Nest, self as *const Nest)
        {
            return Err(NestError::NoRegion);
        }
        Ok(())
    }

    fn check_owner(&self, reservation: &Reservation) -> Result<(), NestError> {
        let owner = reservation.nest().ok_or(NestError::NoRegion)?;
        if !core::ptr::eq(Arc::as_ptr(&owner) as *const Nest, self as *const Nest) {
            return Err(NestError::NoRegion);
        }
        Ok(())
    }

    fn check_reservation(&self, reservation: &Reservation) -> Result<(), NestError> {
        self.check_owner(reservation)?;
        if !self.occupancy.lock().has_id(reservation.id()) {
            return Err(NestError::NoRegion);
        }
        Ok(())
    }

    fn is_closed(&self) -> bool {
        self.state.lock().phase.is_closed()
    }
}

impl Nest {
    /// 翻译里只取物理地址的那一面：权限位不外露。
    pub(crate) fn physical_at(&self, address: VirtAddr) -> Option<PhysAddr> {
        self.translate(address).map(|(physical, _)| physical)
    }

    pub(crate) fn check_access(&self, start: VirtAddr, size: usize, need: Access) -> bool {
        self.access_range(start, size, need, None)
    }

    pub(crate) fn copy_from(&self, start: VirtAddr, dst: &mut [u8], need: Access) -> bool {
        self.access_range(start, dst.len(), need, Some((dst.as_mut_ptr(), false)))
    }

    pub(crate) fn copy_to(&self, start: VirtAddr, src: &[u8], need: Access) -> bool {
        self.access_range(
            start,
            src.len(),
            need,
            Some((src.as_ptr() as *mut u8, true)),
        )
    }

    fn access_range(
        &self,
        start: VirtAddr,
        size: usize,
        need: Access,
        buffer: Option<(*mut u8, bool)>,
    ) -> bool {
        if start.as_usize().checked_add(size).is_none() {
            return false;
        }
        let Ok(state) = self.ready_state() else {
            return false;
        };
        let user = !self.kind.is_supervisor();
        let mut offset = 0;
        while offset < size {
            let at = start.as_usize() + offset;
            let Some(view) = state.index.find(at) else {
                return false;
            };
            let inner = view.state.lock();
            let Some(range) = inner.range_at(at) else {
                return false;
            };
            let page = VirtAddr::from_raw(at & !(PAGE_SIZE - 1));
            let Ok((_, hardware)) = state.root.walk_ref_geometry(page, self.geometry) else {
                return false;
            };
            if !range.current.contains(need)
                || !hardware.contains(need.to_pte() | PteFlags::V)
                || hardware.contains(PteFlags::G)
            {
                return false;
            }
            if user && !hardware.contains(PteFlags::U) {
                return false;
            }
            let count = (PAGE_SIZE - (at & (PAGE_SIZE - 1))).min(size - offset);
            offset += count;
        }
        let Some((buffer, write_to_user)) = buffer else {
            return true;
        };
        offset = 0;
        while offset < size {
            let at = start.as_usize() + offset;
            let Some(view) = state.index.find(at) else {
                unreachable!("Nest held across access validation and copy")
            };
            let _inner = view.state.lock();
            let page = VirtAddr::from_raw(at & !(PAGE_SIZE - 1));
            let (physical, _) = state
                .root
                .walk_ref_geometry(page, self.geometry)
                .expect("validated page table leaf");
            let within = at & (PAGE_SIZE - 1);
            let count = (PAGE_SIZE - within).min(size - offset);
            let physical = physical.as_usize() as *mut u8;
            unsafe {
                if write_to_user {
                    core::ptr::copy(buffer.add(offset), physical.add(within), count);
                } else {
                    core::ptr::copy(physical.add(within), buffer.add(offset), count);
                }
            }
            offset += count;
        }
        true
    }

    pub(crate) fn translate(&self, address: VirtAddr) -> Option<(PhysAddr, PteFlags)> {
        let state = self.ready_state().ok()?;
        state.index.find(address.as_usize())?;
        let (pa, flags) = state.root.walk_ref_geometry(address, self.geometry).ok()?;
        Some((pa + address.offset(), flags))
    }

    #[cfg(debug_assertions)]
    pub(crate) fn leaf_span(&self, address: VirtAddr) -> Option<usize> {
        let state = self.ready_state().ok()?;
        state.index.find(address.as_usize())?;
        state.root.leaf_span_geometry(address, self.geometry)
    }

    /// 用户页错误的完整处置：先还清待办的失效，再按 PTE 与活动库存判定。
    ///
    /// `true` 表示这次访存已经能继续；`false` 交给入口按原样杀任务。
    /// 命中但权限不足直接判死；命中惰性区间才物化一页，再翻译一次确认。
    pub(crate) fn resolve_fault(
        self: &Arc<Self>,
        addr: VirtAddr,
        pc: usize,
        kind: FaultKind,
    ) -> bool {
        if self.has_pending_invalidation() {
            if let Err(error) = self.retry_invalidation() {
                log::error!("page fault waits for invalidation: {:?}", error);
                return false;
            }
        }
        if let Some((_physical, flags)) = self.translate(addr) {
            if satisfies(flags, kind) {
                log::info!("page fault resolved by re-walk: {:?} at {:?}", kind, addr);
                return true;
            }
            log::error!("page fault permission violation: {:?} at {:?}", kind, addr);
            return false;
        }
        if addr.is_user() {
            let Some(view) = self.probe(addr) else {
                log::error!(
                    "unmapped or guard access: {:?} at {:?}, pc={:#x}",
                    kind,
                    addr,
                    pc
                );
                return false;
            };
            let permitted = view.active().is_ok_and(|active| {
                active.iter().any(|range| {
                    range.start <= addr
                        && range.end() >= addr.as_usize()
                        && satisfies_access(range.current, kind)
                })
            });
            if !permitted {
                log::error!("lazy page permission violation: {:?} at {:?}", kind, addr);
                return false;
            }
            let result = view
                .range(
                    PageOffset::new(view.page_index(addr.page_align())),
                    PageCount::new(1).expect("one fault page"),
                )
                .and_then(|selection| self.materialize_active(selection));
            match result {
                Ok(()) => {
                    return self
                        .translate(addr)
                        .is_some_and(|(_, flags)| satisfies(flags, kind));
                }
                Err(error) => {
                    log::error!("failed to materialize page: {:?}", error);
                    return false;
                }
            }
        }
        log::error!("kernel page fault: {:?} at {:?}, pc={:#x}", kind, addr, pc);
        false
    }

    #[cfg(debug_assertions)]
    pub(crate) fn audit(&self) {
        let state = self.state.lock();
        state.index.audit();
        state.index.audit_views(self, &state.root, self.geometry);
        state.root.visit_leaves(
            (self.geometry.levels - 1) as usize,
            0,
            self.geometry,
            &mut |start, physical, flags, size| {
                let end = start
                    .as_usize()
                    .checked_add(size - 1)
                    .expect("leaf virtual interval is representable");
                let view = state
                    .index
                    .find(start.as_usize())
                    .expect("hardware leaf is absent from the nest index");
                let end_view = state
                    .index
                    .find(end)
                    .expect("hardware leaf extends beyond the nest index");
                assert!(
                    Arc::ptr_eq(&view, &end_view),
                    "hardware leaf crosses View identity"
                );
                let inner = view.state.lock();
                assert!(
                    inner.covers(start.as_usize(), end),
                    "hardware leaf crosses an unmapped View range"
                );
                let mut expected: Option<PteFlags> = None;
                for range in inner
                    .active
                    .iter()
                    .filter(|range| range.overlaps(start.as_usize(), end))
                {
                    let leaf = view.leaf_flags(range.current);
                    if let Some(expected) = expected {
                        assert_eq!(
                            leaf.bits(),
                            expected.bits(),
                            "hardware leaf crosses permission state"
                        );
                    } else {
                        expected = Some(leaf);
                    }
                }
                let expected = expected.expect("hardware leaf belongs to a live range");
                let mask = PteFlags::V
                    | PteFlags::R
                    | PteFlags::W
                    | PteFlags::X
                    | PteFlags::U
                    | PteFlags::G;
                assert_eq!(
                    (flags & mask).bits(),
                    (expected & mask).bits(),
                    "hardware PTE permissions differ from View state at {start:?}"
                );
                if inner.source.is_borrowed() {
                    let offset = start
                        .as_usize()
                        .checked_sub(view.initial_start().as_usize())
                        .expect("borrowed leaf precedes View start");
                    let page = inner
                        .source
                        .materialize()
                        .expect("borrowed source materializes")
                        .physical(PageOffset::new(offset / PAGE_SIZE))
                        .expect("borrowed leaf lies inside the source range");
                    assert_eq!(
                        physical.as_usize(),
                        page.as_usize(),
                        "borrowed PTE physical address differs"
                    );
                } else {
                    let first_page = view.page_index(start);
                    let pages = size / PAGE_SIZE;
                    let first = inner
                        .published
                        .partition_point(|(page, _)| *page < first_page);
                    let last_page = first_page + pages;
                    let last = inner
                        .published
                        .partition_point(|(page, _)| *page < last_page);
                    assert_eq!(
                        last - first,
                        pages,
                        "hardware leaf has pages without published Frames"
                    );
                    for (offset, (page, frame)) in inner.published[first..last].iter().enumerate() {
                        assert_eq!(
                            *page,
                            first_page + offset,
                            "published Frame index gap under hardware leaf"
                        );
                        assert_eq!(
                            frame.physical_address(),
                            physical.as_usize() + offset * PAGE_SIZE,
                            "published Frame physical address differs from hardware PTE"
                        );
                    }
                }
            },
        );
    }
}

fn satisfies(flags: PteFlags, kind: FaultKind) -> bool {
    let need = match kind {
        FaultKind::Instruction => PteFlags::X,
        FaultKind::Load => PteFlags::R,
        FaultKind::Store => PteFlags::W,
    };
    flags.contains(PteFlags::V | need)
}

fn satisfies_access(access: Access, kind: FaultKind) -> bool {
    let need = match kind {
        FaultKind::Instruction => Access::EXECUTE,
        FaultKind::Load => Access::READ,
        FaultKind::Store => Access::WRITE,
    };
    access.contains(need)
}

impl Drop for Nest {
    fn drop(&mut self) {
        if !self.published.load(Ordering::Acquire) {
            if !self.asid.is_kernel() {
                Asid::release_unpublished(self.asid);
            }
        } else if self.asid.is_kernel() {
            return;
        } else if !self.asid_released.load(Ordering::Acquire) {
            if self.close().is_err() || identity::deallocate(self.asid).is_err() {
                return;
            }
            self.asid_released.store(true, Ordering::Release);
        }
        unsafe { ManuallyDrop::drop(&mut self.state) };
    }
}
