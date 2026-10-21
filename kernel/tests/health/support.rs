//! 健康用例的外层组合夹具。
//!
//! 线外入口的“来源准备 → 选址 → 挂载”在这里一次写开：每一步都落到真实原语
//! （`Flat` / `Placement` / `Nest::{reserve,attach,detach}`）。
//!
//! 核心用例使用原语组合；线上编解码与授权用例经真实 dispatch 验证。

use alloc::sync::Arc;
use alloc::vec::Vec;
use core::alloc::Layout;

use env::MemoryCall;
use env::MemoryFail;

use crate::core::memory::PAGE_SIZE;
use crate::core::memory::address::{PageCount, PageIndex, PageOffset, VirtAddr};
use crate::core::memory::flat::{Allocation, BorrowedRegion, Flat, FlatRange, Frame};
use crate::core::memory::nest::table::PteFlags;
use crate::core::memory::nest::{
    Access, ActiveRange, Nest, NestError, Placement, RegionId, Reservation, View, ViewRange,
};
use crate::core::work::pend::SourceAuthorization;
use crate::core::work::unit::space::Space;
use crate::core::work::unit::task::Task;

/// 旧 `basic_bytes` 口径：最少 1 字节，向上取整到页；溢出即拒。
fn basic_bytes(raw: usize) -> Result<usize, NestError> {
    raw.max(1)
        .checked_next_multiple_of(PAGE_SIZE)
        .ok_or(NestError::NotAligned)
}

/// 在声明窗口内登记一笔固定跨度（旧 `reserve_at` 的等价写法）。
pub(crate) fn reserve(
    space: &Space,
    start: VirtAddr,
    size: usize,
) -> Result<Reservation, NestError> {
    if size == 0 || start.offset() != 0 || !size.is_multiple_of(PAGE_SIZE) {
        return Err(NestError::NotAligned);
    }
    if !space.nest.window_contains(RegionId::Normal, start, size) {
        return Err(NestError::NoRegion);
    }
    space
        .nest
        .reserve(Placement::exact(RegionId::Normal, start, size)?)
}

/// 选址：`at` 为空时取满足 `align` 的最低空档，否则要它自己也满足 `align`。
pub(crate) fn place(
    nest: &Arc<Nest>,
    size: usize,
    align: usize,
    at: Option<VirtAddr>,
) -> Result<Reservation, NestError> {
    let Some(addr) = at else {
        return nest.reserve(Placement::lowest(RegionId::Normal, size, align)?);
    };
    if addr.as_usize() % align != 0 {
        return Err(NestError::NotAligned);
    }
    if !nest.window_contains(RegionId::Normal, addr, size) {
        return Err(NestError::NoRegion);
    }
    nest.reserve(Placement::exact(RegionId::Normal, addr, size)?)
}

/// 把线外地址 + 字节跨度换成该关系上的身份选择。
fn select(view: &Arc<View>, start: VirtAddr, size: usize) -> Result<ViewRange, NestError> {
    if start.offset() != 0 || size == 0 || !size.is_multiple_of(PAGE_SIZE) {
        return Err(NestError::NotAligned);
    }
    let offset = start
        .as_usize()
        .checked_sub(view.initial_start().as_usize())
        .ok_or(NestError::NoRegion)?
        / PAGE_SIZE;
    let count = PageCount::new(size / PAGE_SIZE).ok_or(NestError::NotAligned)?;
    view.range(PageOffset::new(offset), count)
}

/// 窗口里最低的空档。
pub(crate) fn reserve_low(
    space: &Space,
    region: RegionId,
    size: usize,
) -> Result<Reservation, NestError> {
    space
        .nest
        .reserve(Placement::lowest(region, size, PAGE_SIZE)?)
}

/// 窗口里最高的空档。
pub(crate) fn reserve_high(
    space: &Space,
    region: RegionId,
    size: usize,
) -> Result<Reservation, NestError> {
    space.nest.reserve(Placement::highest(region, size)?)
}

/// 一段已持有的帧做成的来源；帧表先备齐。
pub(crate) fn frames_source(frames: Vec<Arc<Frame>>) -> Result<FlatRange, NestError> {
    let mut indexed = Vec::new();
    indexed
        .try_reserve_exact(frames.len())
        .map_err(|_| NestError::OutOfMemory)?;
    indexed.extend(
        frames
            .into_iter()
            .enumerate()
            .map(|(index, frame)| (PageIndex::new(index), frame)),
    );
    let count = PageCount::new(indexed.len()).ok_or(NestError::NotAligned)?;
    Ok(Flat::from_frames(count, &indexed)?.whole())
}

/// 把调用方已备好的帧挂到一笔既有占用上。
///
/// 占用是调用方交进来的：本步自己的失败（跨度不匹配、来源准备失败）
/// 显式交还这一笔仍 live 的占用，不留给没人持有的票。
pub(crate) fn attach_frames(
    space: &Space,
    reservation: Reservation,
    frames: Vec<Arc<Frame>>,
    ceiling: Access,
    current: Access,
) -> Result<Arc<View>, NestError> {
    if frames.len() * PAGE_SIZE != reservation.size() {
        let _ = space.nest.release_reservation(reservation);
        return Err(NestError::NotAligned);
    }
    let source = match frames_source(frames) {
        Ok(source) => source,
        Err(error) => {
            let _ = space.nest.release_reservation(reservation);
            return Err(error);
        }
    };
    space.nest.attach(reservation, source, ceiling, current)
}

/// 在 `start` 上登记占用并挂上这些帧（旧 `map_frames`）。
pub(crate) fn map_frames(
    space: &Space,
    start: VirtAddr,
    frames: Vec<Arc<Frame>>,
    ceiling: Access,
    current: Access,
) -> Result<Arc<View>, NestError> {
    let size = frames.len() * PAGE_SIZE;
    let source = frames_source(frames)?;
    let reservation = reserve(space, start, size)?;
    space.nest.attach(reservation, source, ceiling, current)
}

/// 懒来源挂到既有占用上（旧 `map_lazy`）。
///
/// 占用是调用方交进来的：来源准备失败显式交还这一笔仍 live 的占用。
pub(crate) fn map_lazy(
    space: &Space,
    reservation: Reservation,
    ceiling: Access,
    current: Access,
) -> Result<Arc<View>, NestError> {
    let source = match lazy_source(reservation.size()) {
        Ok(source) => source,
        Err(error) => {
            let _ = space.nest.release_reservation(reservation);
            return Err(error);
        }
    };
    space.nest.attach(reservation, source, ceiling, current)
}

/// 懒来源：只建立逻辑页，不提前物化。
fn lazy_source(size: usize) -> Result<FlatRange, NestError> {
    Ok(Flat::acquire(Allocation::new(size / PAGE_SIZE, PAGE_SIZE)?)?.whole())
}

/// 旧 `allocate`：返回前完整物化，再登记占用。
pub(crate) fn allocate(
    space: &Space,
    layout: Layout,
    access: Access,
    at: Option<VirtAddr>,
) -> Result<Arc<View>, MemoryFail> {
    let size = basic_bytes(layout.size())?;
    let source = Flat::acquire(Allocation::new(size / PAGE_SIZE, PAGE_SIZE)?)?.whole();
    source.materialize()?;
    space.nest.retry_invalidation()?;
    let reservation = place(&space.nest, size, layout.align().max(PAGE_SIZE), at)?;
    Ok(space
        .nest
        .attach(reservation, source, Access::all(), access)?)
}

/// 旧匿名 `allocate_lazy`：先建来源，再登记占用。
pub(crate) fn allocate_lazy(
    space: &Space,
    layout: Layout,
    limit: Access,
    current: Access,
    at: Option<VirtAddr>,
) -> Result<Arc<View>, MemoryFail> {
    let size = basic_bytes(layout.size())?;
    let source = Flat::acquire(Allocation::new(size / PAGE_SIZE, PAGE_SIZE)?)?.whole();
    space.nest.retry_invalidation()?;
    let reservation = place(&space.nest, size, layout.align().max(PAGE_SIZE), at)?;
    Ok(space.nest.attach(reservation, source, limit, current)?)
}

/// 旧 `attach_static`：调用方仍持的物理区间按给定属性挂到既有占用上。
pub(crate) fn attach_static(
    space: &Space,
    reservation: Reservation,
    region: Arc<BorrowedRegion>,
    offset: usize,
    size: usize,
    access: Access,
    attrs: PteFlags,
) -> Result<Arc<View>, NestError> {
    let start = reservation.start();
    let source = Flat::from_region(region)?.range(
        PageIndex::new(offset / PAGE_SIZE),
        PageCount::new(size / PAGE_SIZE).ok_or(NestError::NotAligned)?,
    )?;
    let active = [ActiveRange::new(start, size, access, access)?];
    space
        .nest
        .attach_shaped(reservation, source, attrs, &active)
}

/// 派生编排的显式组合：旧 `view` / `view_record` / `project` 的等价写法。
///
/// 源活动片段先在来源关系上物化并发布，再按请求的窗口与权限挂到新占用上；
/// 洞不被重发布，派生不继承源页的 current 权限。只服务派生等价用例。
pub(crate) fn derive(
    space: &Space,
    source: &Arc<View>,
    offset: usize,
    size: usize,
    align: usize,
    limit: Access,
    current: Access,
    at: Option<VirtAddr>,
) -> Result<Arc<View>, MemoryFail> {
    if offset % PAGE_SIZE != 0 || size == 0 || !size.is_multiple_of(PAGE_SIZE) {
        return Err(MemoryFail::NotAligned);
    }
    // 整段还是窗口：身份选择在这里就位。
    let selection = if offset == 0 && size == source.initial_size() {
        source.whole()
    } else {
        source
            .range(
                PageOffset::new(offset / PAGE_SIZE),
                PageCount::new(size / PAGE_SIZE).ok_or(MemoryFail::NotAligned)?,
            )
            .map_err(|_| MemoryFail::NoRegion)?
    };
    let view = selection.view().clone();
    let origin = view.owner().ok_or(MemoryFail::NoRegion)?;
    let (lo, _) = selection.window();
    let fragments = fragments_of(&selection)?;
    if fragments.is_empty() {
        return Err(MemoryFail::NoRegion);
    }
    let source_range = selection.source().map_err(|_| MemoryFail::NoRegion)?;
    let mut active = Vec::new();
    active
        .try_reserve(fragments.len())
        .map_err(|_| MemoryFail::OoM)?;
    // 旧副作用：源活动片段先在 Flat 上物化，再回到源关系发布。
    for (start, len) in &fragments {
        let page = (start.as_usize() - view.initial_start().as_usize()) / PAGE_SIZE;
        let count = PageCount::new(len / PAGE_SIZE).ok_or(MemoryFail::NotAligned)?;
        let part = view
            .range(PageOffset::new(page), count)
            .map_err(|_| MemoryFail::NoRegion)?;
        origin
            .materialize_active(part)
            .map_err(|_| MemoryFail::NoRegion)?;
    }
    space.nest.retry_invalidation()?;
    let reservation = place(&space.nest, size, align, at)?;
    // 到这里只剩活动形状的装配：失败就交还本次仍 live 的占用。
    let target = reservation.start().as_usize();
    for (start, len) in fragments {
        match ActiveRange::new(
            VirtAddr::wrap(target + (start.as_usize() - lo)),
            len,
            limit,
            current,
        ) {
            Ok(range) => active.push(range),
            Err(_) => {
                let _ = space.nest.release_reservation(reservation);
                return Err(MemoryFail::NotAligned);
            }
        }
    }
    let attrs = space.nest.attrs(reservation.region());
    Ok(space
        .nest
        .attach_shaped(reservation, source_range, attrs, &active)?)
}

/// 选择窗口内的活动片段（含尾、地址升序）：交集算术用关系自己的原语。
fn fragments_of(selection: &ViewRange) -> Result<Vec<(VirtAddr, usize)>, MemoryFail> {
    let (lo, hi) = selection.window();
    let active = selection
        .view()
        .active()
        .map_err(|_| MemoryFail::NoRegion)?;
    let mut out = Vec::new();
    out.try_reserve(active.len()).map_err(|_| MemoryFail::OoM)?;
    for range in active {
        if let Some((start, end)) = range.intersection(lo, hi) {
            out.push((VirtAddr::from_raw(start), end - start + 1));
        }
    }
    Ok(out)
}

/// 为一段已有关系的**来源范围**登记一份新 ABI 的引用（Pole 的创建来源），
/// 并把这段关系登进目标 Space 的控制记录——Pole 创建就是按它关联 Input 的。
///
/// 这是测试的 setup：新 ABI 的入口只能拿引用，而老夹具手里是直接建好的 `View`。
pub(crate) fn source_reference(
    task: &Arc<Task>,
    space: &Arc<Space>,
    view: &Arc<View>,
) -> env::FlatReference {
    let view_id = view.identity();
    // 这段关系可能已由新 `Attach` 登过（那它就是控制记录的事实）：
    // 直接**复用那份来源授权**，不再第二次 claim 同一个号（否则 Busy）。
    let authorization = match space.views.registered(view_id) {
        Some(entry) => entry.source.clone(),
        None => {
            let range = view.source();
            let authorization =
                SourceAuthorization::new(range.clone(), Access::all(), task.ident.id)
                    .expect("source");
            space
                .views
                .claim(
                    view_id,
                    space,
                    task.ident.id,
                    env::ViewControl::Caller,
                    authorization.clone(),
                    None,
                )
                .expect("claim");
            space.views.commit(view_id, view.clone());
            authorization
        }
    };
    let range = view.source();
    let id = task
        .pend
        .references
        .insert(
            authorization,
            PageOffset::ZERO,
            range.page_count(),
            Access::all(),
            None,
            None,
        )
        .expect("reference");
    match MemoryCall::from_wire((2usize << 32) | 6, &[id, 0, 0, 0, 0, 0]).expect("release") {
        MemoryCall::Release { reference } => reference,
        other => panic!("unexpected release: {other:?}"),
    }
}

pub(crate) fn source_pole(
    task: &Arc<Task>,
    space: &Arc<Space>,
    view: &Arc<View>,
    shared: bool,
) -> (env::PieToken, Arc<crate::core::work::mail::pole::PoleMeta>) {
    let reference = source_reference(task, space, view);
    let token = crate::entry::call::pole::create_pole(task, reference, shared).expect("source Pole");
    task.pend
        .references
        .remove(reference.get())
        .expect("release source reference");
    let meta = crate::core::work::pend::locate(task, token)
        .expect("root Pie")
        .mail()
        .pole()
        .expect("Pole");
    (token, meta)
}

/// 按 VA + 跨度查询已有关系；只用于夹具，不铸造线上身份。
pub(crate) fn view_at(space: &Space, start: VirtAddr, size: usize) -> Result<Arc<View>, NestError> {
    if size == 0 || start.offset() != 0 || !size.is_multiple_of(PAGE_SIZE) {
        return Err(NestError::NoRegion);
    }
    space
        .nest
        .probe(start)
        .filter(|view| view.initial_range() == (start, size) && view.is_live())
        .ok_or(NestError::NoRegion)
}

/// 旧 `unmap_range`：按字节区间逐段换成身份选择再撤。
pub(crate) fn unmap_range(space: &Space, start: VirtAddr, size: usize) -> Result<(), NestError> {
    if !Space::user_range(start, size) || start.offset() != 0 || !size.is_multiple_of(PAGE_SIZE) {
        return Err(NestError::NotAligned);
    }
    let end = start.as_usize() + size;
    let ranges = space.nest.active_ranges()?;
    let mut selections = Vec::new();
    selections
        .try_reserve(ranges.len())
        .map_err(|_| NestError::OutOfMemory)?;
    for (view, range) in ranges {
        let lo = start.as_usize().max(range.start.as_usize());
        if range.start.as_usize() >= end || range.end() < start.as_usize() {
            continue;
        }
        let hi = (range.end() + 1).min(end);
        if lo < hi {
            selections.push(select(&view, VirtAddr::wrap(lo), hi - lo)?);
        }
    }
    if selections.is_empty() {
        return Err(NestError::NoRegion);
    }
    for selection in selections {
        space.nest.detach(selection)?;
    }
    Ok(())
}

/// 旧 `protect_range`：先整体验完覆盖与上限，再按原次序逐段改权。
pub(crate) fn protect(
    space: &Space,
    addr: VirtAddr,
    size: usize,
    access: Access,
) -> Result<(), MemoryFail> {
    if addr.offset() != 0 || !Space::user_range(addr, size) {
        return Err(MemoryFail::NotAligned);
    }
    space.nest.retry_invalidation()?;
    let end = addr.as_usize() + size;
    let ranges = space.nest.active_ranges()?;
    let mut updates = Vec::new();
    updates
        .try_reserve(ranges.len())
        .map_err(|_| MemoryFail::OoM)?;
    let mut covered = addr.as_usize();
    for (view, range) in ranges {
        if range.start.as_usize() >= end || range.end() < addr.as_usize() {
            continue;
        }
        let lo = range.start.as_usize().max(addr.as_usize());
        let hi = (range.end() + 1).min(end);
        if lo != covered {
            return Err(MemoryFail::NoRegion);
        }
        if !range.ceiling.contains(access) {
            return Err(MemoryFail::WidenDenied);
        }
        updates.push((view, VirtAddr::wrap(lo), hi - lo));
        covered = hi;
    }
    if covered != end {
        return Err(MemoryFail::NoRegion);
    }
    for (view, start, size) in updates {
        space.nest.set_access(select(&view, start, size)?, access)?;
    }
    Ok(())
}
