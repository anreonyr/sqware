use alloc::sync::Arc;
use alloc::vec::Vec;

use env::{MemoryCall, MemoryFail};

use crate::core::memory::PAGE_SIZE;
use crate::core::memory::address::{PageCount, PageOffset, VirtAddr as KVirt};
use crate::core::memory::flat::{Allocation, Flat, FlatError, FlatRange};
use crate::core::memory::nest::{
    Access, Nest, NestError, Placement, RegionId, Reservation, View, ViewRange, identity,
};
use crate::core::work::mail::pole::PoleOperation;
use crate::core::work::unit::space::Space;
use crate::core::work::unit::task::TaskIdent;
use crate::machine::switcher::context::TrapContext;

use super::outcome::answer;

pub(super) fn dispatch(frame: &mut TrapContext, call: MemoryCall, ident: &Arc<TaskIdent>) {
    answer(frame, execute(call, ident));
}

fn execute(call: MemoryCall, ident: &Arc<TaskIdent>) -> Result<usize, MemoryFail> {
    use crate::core::work::pend::{self, Permission};
    use crate::core::work::room::conductor::current;
    use crate::core::work::room::scheduler::roster::muster;
    let raw_size = match call {
        MemoryCall::Allocate { size }
        | MemoryCall::Deallocate { size, .. }
        | MemoryCall::Mmap { size, .. }
        | MemoryCall::Munmap { size, .. }
        | MemoryCall::Mprotect { size, .. } => size,
    };
    // 尺寸口径在授权之前：非 basic 报 `NotAligned`，basic 报 `Denied`，两者不换码。
    let basic = matches!(
        call,
        MemoryCall::Allocate { .. } | MemoryCall::Deallocate { .. }
    );
    let size = if basic {
        basic_bytes(raw_size)?
    } else {
        exact_bytes(raw_size)?
    };
    let team_id = match call {
        MemoryCall::Mmap { team, .. }
        | MemoryCall::Munmap { team, .. }
        | MemoryCall::Mprotect { team, .. } => team,
        _ => env::TeamId::new(0),
    };
    let caller = current()
        .running_task()
        .or_else(|| muster(ident.id).and_then(|task| task.upgrade()));
    let target = if team_id.get() == 0 {
        ident.team.clone()
    } else {
        caller
            .as_ref()
            .and_then(|task| task.clan(team_id))
            .ok_or(MemoryFail::Denied)?
    };
    let _construction = if team_id.get() != 0 {
        Some(target.operation().ok_or(MemoryFail::Busy)?)
    } else {
        None
    };
    if team_id.get() != 0 && !target.constructing() {
        return Err(MemoryFail::Denied);
    }
    let space = &target.space;
    let nest = &space.nest;
    match call {
        MemoryCall::Allocate { .. } => {
            let caller = caller.as_ref().ok_or(MemoryFail::Denied)?;
            let prepared = caller
                .pend
                .creations
                .prepare()
                .map_err(|_| MemoryFail::OoM)?;
            // 走字节入口（同一条 basic 口径）：`Layout` 表示不了的线上尺寸不在这里
            // 被强迫失败。旧 `Allocate` 仍是 eager：先把来源物化齐，再登记占用。
            let source = Flat::acquire(Allocation::new(size / PAGE_SIZE, PAGE_SIZE)?)?.whole();
            source.materialize()?;
            nest.retry_invalidation()?;
            let reservation = place(nest, size, PAGE_SIZE, None)?;
            let view = nest.attach(
                reservation,
                source,
                Access::all(),
                Access::READ | Access::WRITE,
            )?;
            let address = view.initial_start().as_usize();
            prepared.commit(&view);
            Ok(address)
        }
        MemoryCall::Deallocate { addr, .. } => {
            if !Space::user_range(KVirt::wrap(addr.get()), size) {
                return Err(MemoryFail::Denied);
            }
            let caller = caller.as_ref().ok_or(MemoryFail::Denied)?;
            let view = caller
                .pend
                .creations
                .locate(KVirt::wrap(addr.get()), size)
                .ok_or(MemoryFail::Denied)?;
            let creation = caller.pend.creations.claim(&view).map_err(|error| {
                if error == env::PieFail::Busy {
                    MemoryFail::Busy
                } else {
                    MemoryFail::Denied
                }
            })?;
            nest.retry_invalidation()?;
            if view.is_live() {
                nest.detach(view.whole())?;
            }
            creation.commit();
            Ok(0)
        }
        MemoryCall::Mmap {
            at,
            pole,
            offset,
            flags,
            ..
        } => {
            let access = Access::from_wire(flags).ok_or(MemoryFail::Denied)?;
            if !access.contains(Access::READ) || access.contains(Access::WRITE | Access::EXECUTE) {
                return Err(MemoryFail::Denied);
            }
            let source = if pole == env::PieToken::NONE {
                None
            } else {
                let caller = caller.as_ref().ok_or(MemoryFail::Denied)?;
                let pie = pend::locate(caller, pole).ok_or(MemoryFail::Denied)?;
                let meta = pie.mail().pole().ok_or(MemoryFail::Denied)?;
                Some((pie, meta))
            };
            let _operation = match &source {
                Some((_, meta)) => Some(PoleOperation::acquire(meta).ok_or(MemoryFail::Busy)?),
                None => None,
            };
            let mut private = false;
            let mut limit = access;
            let mut window = None;
            if let Some((pie, meta)) = &source {
                if !pie.alive() || !meta.owned() || meta.reserved() != 0 {
                    return Err(MemoryFail::Denied);
                }
                let permission = pie.permission();
                if !permission.contains(Permission::FETCH)
                    || access.contains(Access::WRITE) && !permission.contains(Permission::STORE)
                    || team_id.get() != 0 && !permission.contains(Permission::VEST)
                {
                    return Err(MemoryFail::Denied);
                }
                super::pie::usable(pie).map_err(|_| MemoryFail::Denied)?;
                // 授权窗口在来源范围上切：越界与跨页偏移都不放行。
                window = Some(
                    meta.window(offset, size)
                        .map_err(|_| MemoryFail::NotAligned)?,
                );
                if team_id.get() == 0 {
                    if access.contains(Access::EXECUTE) {
                        return Err(MemoryFail::Denied);
                    }
                    limit = if permission.contains(Permission::STORE) {
                        Access::READ | Access::WRITE
                    } else {
                        Access::READ
                    };
                } else if permission.contains(Permission::ONLY) {
                    if pie.sire().is_some()
                        || pie.heir().is_some()
                        || meta.owner() != ident.id
                        || !meta.exclusive()
                        || offset != 0
                        || size != meta.size()
                        || access.contains(Access::EXECUTE)
                        || meta.mapped()
                    {
                        return Err(MemoryFail::Busy);
                    }
                    target.reserve_staging(1).map_err(|_| MemoryFail::OoM)?;
                    private = true;
                } else if !meta.readonly() {
                    return Err(MemoryFail::Busy);
                }
                if team_id.get() == 0 || private {
                    meta.reserve_record().map_err(|_| MemoryFail::OoM)?;
                }
            } else if offset != 0 || access.contains(Access::EXECUTE) {
                return Err(MemoryFail::Denied);
            }
            let at = if at.get() == 0 {
                None
            } else {
                fixed_range(KVirt::wrap(at.get()), size)?;
                Some(KVirt::wrap(at.get()))
            };
            let view = match &source {
                Some(_) => {
                    // 旧共享路径先结算待办失效，再按授权窗口选址挂载。
                    nest.retry_invalidation()?;
                    attach_source(
                        nest,
                        window.ok_or(MemoryFail::Denied)?,
                        PAGE_SIZE,
                        at,
                        limit,
                        access,
                    )?
                }
                None => allocate_lazy(nest, size, limit, access, at)?,
            };
            let address = view.initial_start().as_usize();
            if let Some((pie, _)) = &source {
                let permission = pie.permission();
                if !pie.alive()
                    || !permission.contains(Permission::FETCH)
                    || access.contains(Access::WRITE) && !permission.contains(Permission::STORE)
                {
                    let _ = nest.dispose(view);
                    return Err(MemoryFail::Denied);
                }
            }
            if access.contains(Access::EXECUTE) {
                if let Err(error) = identity::sync_instructions() {
                    let _ = nest.dispose(view);
                    return Err(error.into());
                }
            }
            if let Some((pie, meta)) = &source {
                if private {
                    let staged = view.clone();
                    if let Err(view) = meta.record(pie.token(), space, view, Some(target.id), false)
                    {
                        let _ = nest.dispose(view);
                        return Err(MemoryFail::Denied);
                    }
                    target.push_staging(crate::core::work::unit::team::Staging {
                        token: pie.token(),
                        meta: meta.clone(),
                        view: staged,
                    });
                    pend::notify(ident.id, pole);
                } else if team_id.get() == 0 {
                    if let Err(view) = meta.record(pie.token(), space, view, None, false) {
                        let _ = nest.dispose(view);
                        return Err(MemoryFail::Denied);
                    }
                }
            }
            Ok(address)
        }
        MemoryCall::Munmap { addr, .. } => {
            if team_id.get() == 0 {
                let maps = nest.views_in(KVirt::wrap(addr.get()), size)?;
                if let Some(caller) = &caller {
                    let pies = caller.pend.pies.lock();
                    if maps.iter().any(|view| {
                        pies.iter().any(|pie| {
                            pie.snapshot()
                                .mail()
                                .pole()
                                .is_some_and(|meta| meta.opened(view))
                        })
                    }) {
                        return Err(MemoryFail::Denied);
                    }
                }
            }
            let staged = target
                .find_staging(|item| item.view.initial_range() == (KVirt::wrap(addr.get()), size));
            if let Some(index) = staged {
                let entry = target.staged_at(index).ok_or(MemoryFail::Denied)?;
                let (meta, token) = (entry.meta.clone(), entry.token);
                let _operation = PoleOperation::acquire(&meta).ok_or(MemoryFail::Busy)?;
                unmap_bytes(nest, KVirt::wrap(addr.get()), size)?;
                meta.detach_staging(token, target.id);
                target.remove_staging(index);
                pend::notify(meta.owner(), token);
                return Ok(0);
            }
            if target
                .find_staging(|item| {
                    let (start, len) = item.view.initial_range();
                    addr.get() < start.as_usize() + len
                        && start.as_usize() < addr.get().saturating_add(size)
                })
                .is_some()
            {
                return Err(MemoryFail::Busy);
            }
            unmap_bytes(nest, KVirt::wrap(addr.get()), size)?;
            Ok(0)
        }
        MemoryCall::Mprotect { addr, flags, .. } => {
            fixed_range(KVirt::wrap(addr.get()), size)?;
            let access = Access::from_wire(flags).ok_or(MemoryFail::Denied)?;
            // 空权限与没有 R 的 W 都拒；RWX 仍放行（不与 Mmap 统一成更严的 W^X）。
            let write_without_read =
                access.contains(Access::WRITE) && !access.contains(Access::READ);
            if access.is_empty() || write_without_read {
                return Err(MemoryFail::Denied);
            }
            protect(nest, KVirt::wrap(addr.get()), size, access)?;
            Ok(0)
        }
    }
}

/// basic 口径的线上尺寸：最少 1 字节，向上取整到页；溢出仍是 `Denied`。
///
/// 健康面（`entry::check`）直接调这几个真实入口核对线外口径，
/// 不再各写一份副本；可见性只放宽到 `entry` 父域。
pub(in crate::entry) fn basic_bytes(raw: usize) -> Result<usize, MemoryFail> {
    raw.max(1)
        .checked_next_multiple_of(PAGE_SIZE)
        .ok_or(MemoryFail::Denied)
}

/// 非 basic 口径的线上尺寸：非零且按页对齐，否则 `NotAligned`。
pub(in crate::entry) fn exact_bytes(raw: usize) -> Result<usize, MemoryFail> {
    if raw == 0 || !raw.is_multiple_of(PAGE_SIZE) {
        return Err(MemoryFail::NotAligned);
    }
    Ok(raw)
}

/// 定点区间：`addr` 必须是页地址，且整段落在用户可达范围内。
fn fixed_range(addr: KVirt, size: usize) -> Result<(), MemoryFail> {
    if addr.offset() != 0 || !Space::user_range(addr, size) {
        return Err(MemoryFail::NotAligned);
    }
    Ok(())
}

/// 在声明窗口内登记一笔固定跨度；不动页表、不物化来源。
pub(in crate::entry) fn reserve_at(
    nest: &Arc<Nest>,
    region: RegionId,
    start: KVirt,
    size: usize,
) -> Result<Reservation, NestError> {
    if size == 0 || start.offset() != 0 || !size.is_multiple_of(PAGE_SIZE) {
        return Err(NestError::NotAligned);
    }
    if !nest.window_contains(region, start, size) {
        return Err(NestError::NoRegion);
    }
    nest.reserve(Placement::exact(region, start, size)?)
}

/// 选址：`at` 为空时取满足 `align` 的最低空档，否则要它自己也满足 `align`。
pub(in crate::entry) fn place(
    nest: &Arc<Nest>,
    size: usize,
    align: usize,
    at: Option<KVirt>,
) -> Result<Reservation, NestError> {
    let Some(addr) = at else {
        return nest.reserve(Placement::lowest(RegionId::Normal, size, align)?);
    };
    if addr.as_usize() % align != 0 {
        return Err(NestError::NotAligned);
    }
    reserve_at(nest, RegionId::Normal, addr, size)
}

/// 旧匿名 `Mmap` 的懒挂载：先建来源，再占位，缺页时仍只物化一页。
fn allocate_lazy(
    nest: &Arc<Nest>,
    size: usize,
    limit: Access,
    current: Access,
    at: Option<KVirt>,
) -> Result<Arc<View>, MemoryFail> {
    // 先备好来源，再取占用：来源失败不留下无人接管的地址。
    let source = Flat::acquire(Allocation::new(size / PAGE_SIZE, PAGE_SIZE)?)?.whole();
    nest.retry_invalidation()?;
    let reservation = place(nest, size, PAGE_SIZE, at)?;
    Ok(nest.attach(reservation, source, limit, current)?)
}

/// 把一段已持有的来源范围选址挂上：长度由来源范围决定。
fn attach_source(
    nest: &Arc<Nest>,
    source: FlatRange,
    align: usize,
    at: Option<KVirt>,
    ceiling: Access,
    current: Access,
) -> Result<Arc<View>, MemoryFail> {
    let size = source
        .page_count()
        .checked_bytes()
        .ok_or(MemoryFail::NotAligned)?;
    let reservation = place(nest, size, align, at)?;
    Ok(nest.attach(reservation, source, ceiling, current)?)
}

/// 把线外的地址长度换成该关系上的身份选择。
fn select(view: &Arc<View>, start: KVirt, size: usize) -> Result<ViewRange, NestError> {
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

/// 旧 VA + 字节口径的撤销：逐段换成身份选择，交核心按段退休/收回。
pub(in crate::entry) fn unmap_bytes(
    nest: &Arc<Nest>,
    start: KVirt,
    size: usize,
) -> Result<(), NestError> {
    if !Space::user_range(start, size) || start.offset() != 0 || !size.is_multiple_of(PAGE_SIZE) {
        return Err(NestError::NotAligned);
    }
    let end = start.as_usize() + size;
    let ranges = nest.active_ranges()?;
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
            selections.push(select(&view, KVirt::wrap(lo), hi - lo)?);
        }
    }
    if selections.is_empty() {
        return Err(NestError::NoRegion);
    }
    for selection in selections {
        nest.detach(selection)?;
    }
    Ok(())
}

/// 一次保护一个页对齐区间：先整体验完覆盖与上限，再按原次序逐段改权。
pub(in crate::entry) fn protect(
    nest: &Arc<Nest>,
    addr: KVirt,
    size: usize,
    access: Access,
) -> Result<(), MemoryFail> {
    fixed_range(addr, size)?;
    nest.retry_invalidation()?;
    let end = addr.as_usize() + size;
    let ranges = nest.active_ranges()?;
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
        updates.push((view, KVirt::wrap(lo), hi - lo));
        covered = hi;
    }
    if covered != end {
        return Err(MemoryFail::NoRegion);
    }
    for (view, start, size) in updates {
        nest.set_access(select(&view, start, size)?, access)?;
    }
    if access.contains(Access::EXECUTE) {
        identity::sync_instructions()?;
    }
    Ok(())
}

/// 地址空间失败折成线上码：`Denied` 兼收 `Pinned`、`NotMapped` 与 `DramOverlap`。
impl From<NestError> for MemoryFail {
    fn from(error: NestError) -> Self {
        match error {
            NestError::OutOfMemory => MemoryFail::OoM,
            NestError::NotAligned => MemoryFail::NotAligned,
            NestError::NoRegion => MemoryFail::NoRegion,
            NestError::AlreadyMapped => MemoryFail::AlreadyMapped,
            NestError::WidenDenied => MemoryFail::WidenDenied,
            NestError::Shootdown => MemoryFail::Busy,
            NestError::Pinned => MemoryFail::Denied,
            NestError::NotMapped | NestError::DramOverlap => MemoryFail::Denied,
        }
    }
}

/// 来源失败与地址空间同轴折叠。
impl From<FlatError> for MemoryFail {
    fn from(error: FlatError) -> Self {
        NestError::from(error).into()
    }
}
