use alloc::sync::Arc;

use env::wire::Field;
use env::{AcquireSource, MemoryCall, MemoryFail, Permission, PieToken};

use crate::core::memory::PAGE_SIZE;
use crate::core::memory::address::{PageCount, PageOffset, VirtAddr as KVirt};
use crate::core::memory::flat::{Allocation, Flat, FlatError};
use crate::core::memory::nest::{
    Access, Nest, NestError, Placement, PlacementAt, RegionId, ViewRange, identity,
};
use crate::core::work::mail::Mail;
use crate::core::work::mail::pole::PoleOperation;
use crate::core::work::pend::SourceAuthorization;
use crate::core::work::unit::space::{Space, ViewEntry};
use crate::core::work::unit::task::{Task, TaskIdent};
use crate::core::work::unit::team::Team;
use crate::machine::switcher::context::TrapContext;

use super::outcome::{answer, answer_pair};

#[cfg(debug_assertions)]
static FAIL_PREPARE: core::sync::atomic::AtomicU8 = core::sync::atomic::AtomicU8::new(0);

#[cfg(debug_assertions)]
pub(crate) fn fail_prepare_at(slot: u8) {
    FAIL_PREPARE.store(slot, core::sync::atomic::Ordering::Relaxed);
}

fn prepare_checkpoint(slot: u8) -> Result<(), MemoryFail> {
    #[cfg(debug_assertions)]
    if FAIL_PREPARE
        .compare_exchange(
            slot,
            0,
            core::sync::atomic::Ordering::Relaxed,
            core::sync::atomic::Ordering::Relaxed,
        )
        .is_ok()
    {
        return Err(MemoryFail::OoM);
    }
    let _ = slot;
    Ok(())
}

pub(super) fn dispatch(frame: &mut TrapContext, call: MemoryCall, ident: &Arc<TaskIdent>) {
    match call {
        MemoryCall::Attach {
            range,
            team,
            placement,
            access,
            control,
        } => {
            answer_pair(
                frame,
                attach(range, team, placement, access, control, ident),
            );
        }
        _ => answer(frame, execute(call, ident)),
    }
}

fn execute(call: MemoryCall, ident: &Arc<TaskIdent>) -> Result<usize, MemoryFail> {
    match call {
        MemoryCall::Acquire { source, retention } => acquire(source, retention, ident),
        MemoryCall::Release { reference } => release(reference, ident),
        MemoryCall::Detach {
            team,
            view,
            selection,
        } => detach(team, view, selection, ident),
        MemoryCall::SetAccess {
            team,
            view,
            selection,
            access,
        } => set_access(team, view, selection, access, ident),
        MemoryCall::Traverse {
            retention,
            range,
            out,
        } => traverse(retention, range, out, ident),
        MemoryCall::Attach { .. } => unreachable!("Attach answers a pair"),
    }
}

/// 目标域：当前域，或调用者的 Constructing 子域（非零 Team 还要求仍在构造中）。
fn resolve_team(
    team: env::TeamId,
    ident: &Arc<TaskIdent>,
    caller: &Arc<Task>,
) -> Result<Arc<Team>, MemoryFail> {
    if team.get() == 0 {
        return Ok(ident.team.clone());
    }
    let target = caller.clan(team).ok_or(MemoryFail::Denied)?;
    if !target.constructing() {
        return Err(MemoryFail::Denied);
    }
    Ok(target)
}

/// 从**调用者**地址空间一次快照一段定长只读描述。
fn read_snapshot<const N: usize>(space: &Space, at: KVirt) -> Result<[u8; N], MemoryFail> {
    let mut bytes = [0; N];
    if !space.copy_in(&mut bytes, at) {
        return Err(MemoryFail::Denied);
    }
    Ok(bytes)
}

/// 把线外的选址描述换成 Nest 的选址；长度不在这里——由来源范围定。
fn nest_placement(
    nest: &Nest,
    placement: env::Placement,
    size: usize,
) -> Result<Placement, MemoryFail> {
    let at = match placement.at() {
        env::PlacementAt::Lowest => PlacementAt::Lowest,
        env::PlacementAt::Highest => PlacementAt::Highest,
        env::PlacementAt::Exact(address) => {
            let address = KVirt::wrap(address.get());
            if !Space::user_range(address, size) {
                return Err(MemoryFail::NotAligned);
            }
            if !nest.window_contains(RegionId::Normal, address, size) {
                return Err(MemoryFail::NoRegion);
            }
            PlacementAt::Exact(address)
        }
    };
    Placement::new(RegionId::Normal, size, placement.align(), at).map_err(MemoryFail::from)
}

fn cancel_view(space: &Space, task: &Task, id: usize) {
    space.views.cancel(id);
    task.release_control(id);
}

/// `Attach`：预留地址并建立翻译关系，返回 `(关系句柄, 视图起点)`。
///
/// 发布与撤销共取来源授权上那一把闸门：发布前确认没被撤销，发布后再复查一次；
/// 撤销方在同一把门下置位。于是撤销要么看见这次发布、要么发布自己发现已被撤销。
fn attach(
    range_at: env::VirtAddr,
    team: env::TeamId,
    placement_at: env::VirtAddr,
    access: u64,
    control: env::ViewControl,
    ident: &Arc<TaskIdent>,
) -> Result<(usize, usize), MemoryFail> {
    let caller_task = caller(ident).ok_or(MemoryFail::Denied)?;
    let caller_space = ident.team.space.clone();
    let range = read_snapshot::<{ env::ReferenceRange::WIDTH }>(
        &caller_space,
        KVirt::wrap(range_at.get()),
    )?;
    let placement =
        read_snapshot::<{ env::Placement::WIDTH }>(&caller_space, KVirt::wrap(placement_at.get()))?;
    let range = env::ReferenceRange::fetch(&range).ok_or(MemoryFail::NotAligned)?;
    let placement = match env::Placement::decode_bytes(&placement) {
        Ok(placement) => placement,
        // 方式标签不认得：整段描述不是本 API 的请求。
        Err(env::PlacementError::Tag) => return Err(MemoryFail::Denied),
        // 方式认得而对齐/定点非法：格式可解，参数不合法。
        Err(env::PlacementError::Align) => return Err(MemoryFail::NotAligned),
    };
    let access = Access::from_wire(access).ok_or(MemoryFail::Denied)?;
    // 空权限拒。执行权不由 Pie 权限位（更不由 VEST）表达：它只随**目标域授权**
    // 与来源上限走，所以放到 `resolve_team` 之后判。
    if access.is_empty() {
        return Err(MemoryFail::Denied);
    }
    let target = resolve_team(team, ident, &caller_task)?;
    let _construction = if team.get() != 0 {
        Some(target.operation().ok_or(MemoryFail::Busy)?)
    } else {
        None
    };
    if team.get() != 0 && !target.constructing() {
        return Err(MemoryFail::Denied);
    }
    // 当前域不开执行权（旧 `Allocate` 给的也是 RW）；调用者的 Constructing
    // 子域按上限开——装载器就是走这条路装代码段。
    if access.contains(Access::EXECUTE) && team.get() == 0 {
        return Err(MemoryFail::Denied);
    }
    let space = target.space.clone();
    // **本域**还是**交付到别的域**：交付出去的关系不算调用者的映射，不得挂在调用者
    // 那条可撤链上（否则来源回收 / 撤销会把别人正在跑的代码拆掉）。
    let own_space = team.get() == 0;
    // 引用表下只取一份合法持有快照，随后立刻放锁：核心工作（Nest 同步 / 挂载）
    // 不得带着资源表锁等待；并发的 Release 只会把它从表里摘走，这份快照仍保有来源。
    let hold = {
        let _serial = caller_task.pend.lock();
        caller_task
            .pend
            .references
            .get(range.reference.get())
            .ok_or(MemoryFail::Denied)?
    };
    let count = PageCount::new(range.count).ok_or(MemoryFail::NotAligned)?;
    let source = hold
        .resolve()
        .ok_or(MemoryFail::Denied)?
        .range(PageOffset::new(range.offset), count)
        .map_err(|_| MemoryFail::NoRegion)?;
    let size = count.checked_bytes().ok_or(MemoryFail::NotAligned)?;
    let authorization = hold.source().clone();
    let root = authorization.root();
    let branch = hold
        .branch()
        .map(|(token, _)| token)
        .or_else(|| root.as_ref().map(|(_, token)| *token));
    let pole = root.map(|(meta, _)| meta);
    let grant = hold.grant().cloned().or_else(|| {
        pole.as_ref()
            .zip(branch)
            .and_then(|(meta, token)| meta.grant(token))
    });
    let _operation = pole
        .as_ref()
        .map(|meta| PoleOperation::acquire(meta).ok_or(MemoryFail::Busy))
        .transpose()?;
    let permission = match &grant {
        Some(grant) => grant.permission(),
        None => authorization.permission(),
    }
    .ok_or(MemoryFail::Denied)?;
    if !permission.contains(Permission::FETCH)
        || (access.contains(Access::WRITE) && !permission.contains(Permission::STORE))
        || (!own_space && !permission.contains(Permission::VEST))
    {
        return Err(MemoryFail::Denied);
    }
    if !hold.ceiling().contains(access) {
        return Err(MemoryFail::WidenDenied);
    }
    let mut ceiling = hold.ceiling();
    if !permission.contains(Permission::STORE) {
        ceiling.remove(Access::WRITE);
    }
    if !own_space {
        ceiling &= access;
    } else if pole.is_some() {
        ceiling &= Access::READ | Access::WRITE;
    }
    let mut staged = false;
    if let Some(meta) = &pole {
        if !meta.alive() || meta.reserved() != 0 {
            return Err(MemoryFail::Busy);
        }
        if !own_space {
            if permission.contains(Permission::ONLY) {
                let token = branch.ok_or(MemoryFail::Denied)?;
                let pie = crate::core::work::pend::locate(&caller_task, token)
                    .ok_or(MemoryFail::Denied)?;
                if pie.sire().is_some()
                    || pie.heir().is_some()
                    || meta.owner() != ident.id
                    || !meta.exclusive()
                    || !source.same_range(&meta.source())
                    || access.contains(Access::EXECUTE)
                    || meta.mapped()
                {
                    return Err(MemoryFail::Busy);
                }
                target.reserve_staging(1)?;
                staged = true;
            } else if !meta.readonly() || access.contains(Access::WRITE) {
                return Err(MemoryFail::Busy);
            }
        }
    } else {
        if !authorization.private_owner(ident.id) {
            return Err(MemoryFail::Denied);
        }
        if !own_space && access.contains(Access::EXECUTE) {
            return Err(MemoryFail::Denied);
        }
    }
    let nest = &space.nest;
    nest.retry_invalidation()?;
    space.reap_views();
    // 分支下的登记位先备好（只在本域安装时才需要）：提交时只替换，不再可能失败。
    if (own_space || staged)
        && let Some(meta) = &pole
    {
        meta.reserve_records(1).map_err(source_fail)?;
    }
    prepare_checkpoint(7)?;
    let reservation = nest.reserve(nest_placement(nest, placement, size)?)?;
    let id = reservation.id();
    // 号必须在符号上界内：过界就当场交还这笔占用，不拿它去写 PTE。
    if id > isize::MAX as usize {
        nest.release_reservation(reservation)?;
        return Err(MemoryFail::OoM);
    }
    // 发布前先占住登记槽与在途号：两处的容量都已备齐，提交只替换已备位置。
    if (own_space || pole.is_none())
        && let Err(error) = authorization.remember_space(&space)
    {
        let _ = nest.release_reservation(reservation);
        return Err(source_fail(error));
    }
    if let Err(error) =
        space
            .views
            .claim(id, &space, ident.id, control, authorization.clone(), branch)
    {
        let _ = nest.release_reservation(reservation);
        return Err(source_fail(error));
    }
    if control == env::ViewControl::Caller
        && let Err(error) = caller_task.track_control(&space, id)
    {
        space.views.cancel(id);
        let _ = nest.release_reservation(reservation);
        return Err(error.into());
    }
    if let Err(error) = authorization.enter_publish(id, grant.as_deref(), access) {
        cancel_view(&space, &caller_task, id);
        let _ = nest.release_reservation(reservation);
        return Err(source_fail(error));
    }
    let view = match nest.attach(reservation, source, ceiling, access) {
        Ok(view) => view,
        Err(error) => {
            authorization.leave_publish(id);
            cancel_view(&space, &caller_task, id);
            return Err(error.into());
        }
    };
    let address = view.initial_start().as_usize();
    // 这段关系带执行权：取完指令缓存才算可用。
    if access.contains(Access::EXECUTE)
        && let Err(error) = identity::sync_instructions()
    {
        cancel_view(&space, &caller_task, id);
        let _ = nest.dispose(view);
        authorization.leave_publish(id);
        return Err(error.into());
    }
    let committed = (|| {
        let _serial = caller_task.pend.lock();
        let _commit = crate::core::work::commit();
        if !crate::core::work::pend::live(&caller_task)
            || !authorization.branch_check(grant.as_deref(), access)
        {
            return Err(MemoryFail::Denied);
        }
        if (own_space || staged)
            && let (Some(meta), Some(token)) = (&pole, branch)
            && meta
                .record(
                    token,
                    &space,
                    view.clone(),
                    staged.then_some(target.id),
                    false,
                )
                .is_err()
        {
            return Err(MemoryFail::Denied);
        }
        if staged {
            target.push_staging(crate::core::work::unit::team::Staging {
                token: branch.expect("staged root token"),
                meta: pole.as_ref().expect("staged Pole").clone(),
                view: view.clone(),
            });
        }
        space.views.commit(id, view.clone());
        Ok(())
    })();
    if let Err(error) = committed {
        cancel_view(&space, &caller_task, id);
        let _ = nest.dispose(view);
        authorization.leave_publish(id);
        return Err(error);
    }
    if staged {
        crate::core::work::pend::notify(ident.id, branch.expect("staged root token"));
    }
    authorization.leave_publish(id);
    Ok((id, address))
}

/// Traverse requires both current authority and a live residency.
///
/// 凭据是**当场拿得出的那枚真保留授权**：只看这份引用“是怎么来的”不够——
/// provenance 只说明存活，不自动赋物理坐标。两层都要过：
/// 活保留 cap（`require_retention`）＋ 这份引用本身就是 cap 登记出来的 retained 引用。
/// 整张表先物化齐了才写出去——失败不在调用者缓冲里留半张。
fn traverse(
    retention: PieToken,
    range_at: env::VirtAddr,
    out: env::VirtAddr,
    ident: &Arc<TaskIdent>,
) -> Result<usize, MemoryFail> {
    let task = caller(ident).ok_or(MemoryFail::Denied)?;
    require_retention(&task, retention)?;
    let space = ident.team.space.clone();
    let bytes =
        read_snapshot::<{ env::ReferenceRange::WIDTH }>(&space, KVirt::wrap(range_at.get()))?;
    let range = env::ReferenceRange::fetch(&bytes).ok_or(MemoryFail::NotAligned)?;
    // 引用表下只取一份合法持有快照，随后立刻放锁：物化与写缓冲都不带着资源表锁做。
    let hold = {
        let _serial = task.pend.lock();
        task.pend
            .references
            .get(range.reference.get())
            .ok_or(MemoryFail::Denied)?
    };
    if !hold.retains() {
        return Err(MemoryFail::Denied);
    }
    let count = PageCount::new(range.count).ok_or(MemoryFail::NotAligned)?;
    let source = hold
        .resolve()
        .ok_or(MemoryFail::Denied)?
        .range(PageOffset::new(range.offset), count)
        .map_err(|_| MemoryFail::NoRegion)?;
    let pages = source.materialize().map_err(MemoryFail::from)?;
    let mut list = alloc::vec::Vec::new();
    list.try_reserve_exact(count.get())
        .map_err(|_| MemoryFail::OoM)?;
    for index in 0..count.get() {
        let address = pages
            .physical(PageOffset::new(index))
            .map_err(MemoryFail::from)?;
        list.push(address.as_usize() as u64);
    }
    let mut bytes = alloc::vec::Vec::new();
    bytes
        .try_reserve_exact(count.get() * 8)
        .map_err(|_| MemoryFail::OoM)?;
    for address in list {
        bytes.extend_from_slice(&address.to_le_bytes());
    }
    // 整张表都备好了才提出口：此处失败不碰调用者缓冲，保留仍在。
    prepare_checkpoint(11)?;
    if !space.copy_out(&bytes, KVirt::wrap(out.get())) {
        return Err(MemoryFail::Denied);
    }
    Ok(0)
}

/// 按目标域与号取回控制登记；号本身不授予任何东西。
fn with_view<T>(
    team: env::TeamId,
    view: env::ViewId,
    ident: &Arc<TaskIdent>,
    action: impl FnOnce(&Arc<Team>, &ViewEntry) -> Result<T, MemoryFail>,
) -> Result<T, MemoryFail> {
    let caller_task = caller(ident).ok_or(MemoryFail::Denied)?;
    let target = resolve_team(team, ident, &caller_task)?;
    let _construction = if team.get() != 0 {
        Some(target.operation().ok_or(MemoryFail::Busy)?)
    } else {
        None
    };
    if team.get() != 0 && !target.constructing() {
        return Err(MemoryFail::Denied);
    }
    let space = target.space.clone();
    let entry = space.views.locate(view.get()).ok_or(MemoryFail::Denied)?;
    let owned = entry
        .space
        .upgrade()
        .is_some_and(|space| Arc::ptr_eq(&space, &target.space));
    if !owned {
        return Err(MemoryFail::Denied);
    }
    let allowed = match entry.scope {
        env::ViewControl::Caller => entry.controller == ident.id,
        // 显式把这一段交给本 Space 成员；不是“同 Space 自动全控”。
        env::ViewControl::SpaceMembers => Arc::ptr_eq(&ident.team.space, &space),
    };
    if !allowed {
        return Err(MemoryFail::Denied);
    }
    action(&target, &entry)
}

/// 把线外的选择换成这一段关系上的身份选择。
fn selection(entry: &ViewEntry, selection: env::ViewSelection) -> Result<ViewRange, MemoryFail> {
    let view = entry.view().ok_or(MemoryFail::Denied)?;
    match selection {
        env::ViewSelection::Whole => Ok(view.whole()),
        env::ViewSelection::Pages { offset, count } => {
            let count = PageCount::new(count).ok_or(MemoryFail::NotAligned)?;
            view.range(PageOffset::new(offset), count)
                .map_err(|_| MemoryFail::NoRegion)
        }
    }
}

fn detach(
    team: env::TeamId,
    view: env::ViewId,
    choice: env::ViewSelection,
    ident: &Arc<TaskIdent>,
) -> Result<usize, MemoryFail> {
    with_view(team, view, ident, |target, entry| {
        let space = &target.space;
        let staged = target.find_staging(|item| item.view.identity() == view.get());
        if staged.is_some() && !matches!(choice, env::ViewSelection::Whole) {
            return Err(MemoryFail::Busy);
        }
        prepare_checkpoint(8)?;
        space.nest.retry_invalidation()?;
        if space.nest.contains_reservation(view.get()) {
            space.nest.detach(selection(entry, choice)?)?;
        }
        if !space.nest.contains_reservation(view.get()) {
            space.retire_view(view.get());
            if let Some(at) = staged {
                let item = target.staged_at(at).ok_or(MemoryFail::Denied)?;
                item.meta.detach_staging(item.token, target.id);
                target.remove_staging(at);
                crate::core::work::pend::notify(item.meta.owner(), item.token);
            }
        }
        Ok(0)
    })
}

fn set_access(
    team: env::TeamId,
    view: env::ViewId,
    choice: env::ViewSelection,
    access: u64,
    ident: &Arc<TaskIdent>,
) -> Result<usize, MemoryFail> {
    let access = Access::from_wire(access).ok_or(MemoryFail::Denied)?;
    if !access.is_leaf_set() {
        return Err(MemoryFail::Denied);
    }
    with_view(team, view, ident, |target, entry| {
        prepare_checkpoint(9)?;
        target.space.nest.retry_invalidation()?;
        target
            .space
            .nest
            .set_access(selection(entry, choice)?, access)?;
        if access.contains(Access::EXECUTE) {
            identity::sync_instructions()?;
        }
        Ok(0)
    })
}

/// 发起调用的那个任务（正在跑的，或者是这个 ident 对应的）。
fn caller(ident: &Arc<TaskIdent>) -> Option<Arc<Task>> {
    use crate::core::work::room::conductor::current;
    use crate::core::work::room::scheduler::roster::muster;
    current()
        .running_task()
        .or_else(|| muster(ident.id).and_then(|task| task.upgrade()))
}

/// 来源侧的失败码折成地址空间同轴的那一条。
fn source_fail(error: env::PieFail) -> MemoryFail {
    match error {
        env::PieFail::OoM => MemoryFail::OoM,
        env::PieFail::Busy => MemoryFail::Busy,
        env::PieFail::NotAligned => MemoryFail::NotAligned,
        _ => MemoryFail::Denied,
    }
}

/// 独立保留授权：只认内核铸的那类真对象（持有者自己的、活的 Pie）。
/// 普通自造 Nole、`Mark`、只有 FETCH/STORE/VEST 都不算。
fn require_retention(task: &Arc<Task>, token: PieToken) -> Result<(), MemoryFail> {
    let pie = crate::core::work::pend::locate(task, token).ok_or(MemoryFail::Denied)?;
    if !pie.alive() || !pie.permission().contains(Permission::VEST) {
        return Err(MemoryFail::Denied);
    }
    let Mail::Nole(meta) = pie.mail() else {
        return Err(MemoryFail::Denied);
    };
    if !meta.retains() || meta.owner() != task.ident.id {
        return Err(MemoryFail::Denied);
    }
    Ok(())
}

/// 登记一份来源持有，返回它在**调用任务**引用表里的句柄。
///
/// 新的 `Allocation` 建立来源持有与一份一次性创建资格；已有引用的子范围与它
/// **共享**同一份来源授权（同一来源、同一上限、同一撤销依赖），不复制资格，
/// 也不因 VA 可读就授权。
fn acquire(
    source: AcquireSource,
    retention: PieToken,
    ident: &Arc<TaskIdent>,
) -> Result<usize, MemoryFail> {
    let task = caller(ident).ok_or(MemoryFail::Denied)?;
    let _serial = task.pend.lock();
    let (authorization, offset, count, ceiling, retained, branch) = match source {
        AcquireSource::Allocation(allocation) => {
            // 新建来源本身就没有旧授权可挂，独立保留授权对它无意义。
            if retention != PieToken::NONE || allocation.align != PAGE_SIZE {
                return Err(MemoryFail::NotAligned);
            }
            let size = basic_bytes(allocation.size)?;
            let pages = size / PAGE_SIZE;
            let count = PageCount::new(pages).ok_or(MemoryFail::NotAligned)?;
            let range = Flat::acquire(Allocation::new(pages, PAGE_SIZE)?)?.whole();
            let authorization =
                SourceAuthorization::new(range, Access::all(), ident.id).map_err(source_fail)?;
            let ceiling = authorization.ceiling();
            (authorization, PageOffset::ZERO, count, ceiling, None, None)
        }
        AcquireSource::Reference(window) => {
            let hold = task
                .pend
                .references
                .get(window.reference.get())
                .ok_or(MemoryFail::Denied)?;
            let count = PageCount::new(window.count).ok_or(MemoryFail::NotAligned)?;
            let range = hold
                .resolve()
                .ok_or(MemoryFail::Denied)?
                .range(PageOffset::new(window.offset), count)
                .map_err(|_| MemoryFail::NoRegion)?;
            let (parent, _) = hold.window();
            let offset = PageOffset::new(
                parent
                    .get()
                    .checked_add(window.offset)
                    .ok_or(MemoryFail::NotAligned)?,
            );
            // 独立保留：此刻把这段范围从当前授权上解下来自己强持，
            // 于是它跨 Grant 撤销继续持同一批帧；不带保留的仍随授权生死。
            let retained = if retention == PieToken::NONE {
                None
            } else {
                require_retention(&task, retention)?;
                Some(range.reside()?)
            };
            (
                hold.source().clone(),
                offset,
                count,
                hold.ceiling(),
                retained,
                hold.branch().map(|(token, grant)| (token, grant.clone())),
            )
        }
        AcquireSource::Source {
            token,
            offset,
            count,
        } => {
            // 从 Pole 的某一枚 Pie 取得：撤销边与上限按**这一枚**的子 Grant 算，
            // 不因同 Flat、共享根资格就并到根上去。
            let pie = crate::core::work::pend::locate(&task, token).ok_or(MemoryFail::Denied)?;
            if !pie.alive() {
                return Err(MemoryFail::Denied);
            }
            let meta = pie.mail().pole().ok_or(MemoryFail::Denied)?;
            if meta.reserved() != 0 {
                return Err(MemoryFail::Denied);
            }
            if !pie.permission().contains(Permission::FETCH) {
                return Err(MemoryFail::Denied);
            }
            let authorization = meta.authorization().ok_or(MemoryFail::Denied)?;
            let ceiling = authorization.ceiling();
            // 分支就是调用者手里这一枚 Pie 的 token 与它的 Grant。
            let grant = pie.grant().clone();
            let (offset, count) = self::window(&authorization, offset, count)?;
            let retained = if retention == PieToken::NONE {
                None
            } else {
                require_retention(&task, retention)?;
                Some(
                    meta.source()
                        .range(offset, count)
                        .map_err(|_| MemoryFail::NoRegion)?
                        .reside()?,
                )
            };
            (
                authorization,
                offset,
                count,
                ceiling,
                retained,
                Some((token, grant)),
            )
        }
        AcquireSource::View {
            team,
            view,
            offset,
            count,
        } => with_view(team, view, ident, |_target, entry| {
            // 从已授权的关系取得它的来源范围；域、所属关系与来源授权都要核，
            // ViewId 或可读 VA 本身不是凭证。
            let view = entry.view().ok_or(MemoryFail::Denied)?.clone();
            let authorization = entry.source.clone();
            let count = PageCount::new(count).ok_or(MemoryFail::NotAligned)?;
            let source = view
                .source()
                .range(PageOffset::new(offset), count)
                .map_err(|_| MemoryFail::NoRegion)?;
            let base = authorization.index().ok_or(MemoryFail::Denied)?;
            let offset = PageOffset::new(
                source
                    .index()
                    .get()
                    .checked_sub(base.get())
                    .ok_or(MemoryFail::NotAligned)?,
            );
            let (offset, count) = self::window(&authorization, offset.get(), count.get())?;
            let ceiling = entry
                .view()
                .map_or(Access::all(), |_| authorization.ceiling());
            let branch = if let Some((meta, root_token)) = authorization.root() {
                let token = entry.branch.unwrap_or(root_token);
                let pie =
                    crate::core::work::pend::locate(&task, token).ok_or(MemoryFail::Denied)?;
                let grant = meta.grant(token).ok_or(MemoryFail::Denied)?;
                if !pie.alive()
                    || !pie.permission().contains(Permission::FETCH)
                    || !Arc::ptr_eq(pie.grant(), &grant)
                {
                    return Err(MemoryFail::Denied);
                }
                Some((token, grant))
            } else {
                if !authorization.private_owner(ident.id) {
                    return Err(MemoryFail::Denied);
                }
                None
            };
            let retained = if retention == PieToken::NONE {
                None
            } else {
                require_retention(&task, retention)?;
                Some(source.reside()?)
            };
            Ok((authorization, offset, count, ceiling, retained, branch))
        })?,
    };
    prepare_checkpoint(5)?;
    task.pend
        .references
        .insert(authorization, offset, count, ceiling, retained, branch)
        .map_err(source_fail)
}

/// 把线外的页窗口换成相对来源整段的坐标，并核它落在来源内。
fn window(
    authorization: &Arc<SourceAuthorization>,
    offset: usize,
    count: usize,
) -> Result<(PageOffset, PageCount), MemoryFail> {
    let count = PageCount::new(count).ok_or(MemoryFail::NotAligned)?;
    let offset = PageOffset::new(offset);
    if authorization
        .full_range()
        .is_none_or(|range| range.range(offset, count).is_err())
    {
        return Err(MemoryFail::NoRegion);
    }
    Ok((offset, count))
}

/// 解除这一份引用：只摘掉本表里这一格，不 Detach、不等其他引用、不分配。
/// 授权已经失效也不妨碍清掉自己的句柄。
fn release(reference: env::FlatReference, ident: &Arc<TaskIdent>) -> Result<usize, MemoryFail> {
    let task = caller(ident).ok_or(MemoryFail::Denied)?;
    let _serial = task.pend.lock();
    task.pend
        .references
        .remove(reference.get())
        .map(|_| 0)
        .ok_or(MemoryFail::Denied)
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
            // 承载已移出：折成既有的“没有所属区域”，不给新线上码，
            // 也不当可无限重试的 Busy。
            NestError::Missing => MemoryFail::NoRegion,
            NestError::Busy => MemoryFail::Busy,
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
