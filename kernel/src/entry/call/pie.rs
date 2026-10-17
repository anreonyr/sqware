use alloc::sync::Arc;
use alloc::vec::Vec;

use env::{
    FlatReference, HoleLimits, Mark, PieCall, PieFail, PieInfo, PieToken, ReleaseMode, TaskId,
    UnsealArgs, VirtAddr,
};

use crate::core::memory::address::VirtAddr as KVirt;
use crate::core::memory::nest::Access;
use crate::core::work::mail::{self, Mail, pole::PoleOperation};
use crate::core::work::pend::{self, Need, Permission, PieSnapshot, clear_heir};
use crate::core::work::room::conductor::current;
use crate::core::work::room::scheduler::roster::muster;
use crate::core::work::unit::task::TaskIdent;
use crate::machine::switcher::context::TrapContext;

use super::outcome::{answer, answer_pair};

fn subset_to_access(subset: Permission) -> Result<Access, PieFail> {
    if !subset.contains(Permission::FETCH) {
        return Err(PieFail::Denied);
    }
    let mut access = Access::READ;
    if subset.contains(Permission::STORE) {
        access |= Access::WRITE;
    }
    Ok(access)
}

pub(crate) fn dispatch(frame: &mut TrapContext, call: PieCall, ident: Arc<TaskIdent>) {
    match call {
        PieCall::Unseal { args } => match args {
            UnsealArgs::Hole { mark, limits } => unseal_hole(frame, mark, limits),
            UnsealArgs::Pole { reference, shared } => unseal_pole(frame, reference, shared),
            UnsealArgs::Nole => unseal_nole(frame),
            UnsealArgs::Tole { shared } => unseal_tole(frame, shared),
        },
        PieCall::Open { token } => open(frame, ident, token),
        PieCall::Shut { token } => shut(frame, ident, token),
        PieCall::Seal { token } => seal(frame, token),
        PieCall::Accord {
            src,
            dst,
            subset,
            mark,
        } => accord(frame, src, dst, subset, mark),
        PieCall::Narrow { token, subset } => narrow(frame, token, subset),
        PieCall::Revoke { dst, token } => revoke(frame, dst, token),
        PieCall::Collect {
            after,
            buf,
            capacity,
        } => collect(frame, &ident, after, buf, capacity),
        PieCall::Inspect { token, buf } => inspect(frame, &ident, token, buf),
        PieCall::Release { token, mode } => release(frame, token, mode),
        PieCall::Same { a, b } => {
            answer(
                frame,
                current()
                    .running_task()
                    .ok_or(PieFail::Denied)
                    .and_then(|task| pend::same(&task, a, b))
                    .map(usize::from),
            );
        }
    }
}

pub(crate) fn usable(pie: &PieSnapshot) -> Result<(), env::PieFail> {
    pend::available(pie).map_err(|_| env::PieFail::HandedOver)?;
    let Some(h) = pie.heir().copied() else {
        return Ok(());
    };
    if let Some(task) = current().running_task() {
        clear_heir(&task, pie.token(), h);
    }
    Ok(())
}

fn unseal_hole(frame: &mut TrapContext, mark: Mark, limits: HoleLimits) {
    let r = (|| -> Result<usize, PieFail> {
        let task = current().running_task().ok_or(PieFail::Denied)?;
        if !limits.valid() {
            return Err(PieFail::Denied);
        }
        let meta = mail::hole::HoleMeta::try_new_with_limits(task.ident.id, limits)
            .map_err(|_| PieFail::OoM)?;
        let pie = pend::Pie::try_new(
            Mail::Hole(meta),
            mark,
            Permission::FETCH | Permission::STORE | Permission::VEST,
            None,
        )?;
        let token = pie.token();
        pend::insert(&task, pie)?;
        Ok(token.get())
    })();
    answer(frame, r);
}

fn unseal_nole(frame: &mut TrapContext) {
    let r = (|| -> Result<usize, PieFail> {
        let task = current().running_task().ok_or(PieFail::Denied)?;
        let meta = mail::nole::NoleMeta::try_new(task.ident.id).map_err(|_| PieFail::OoM)?;
        let pie = pend::Pie::try_new(
            Mail::Nole(meta),
            Mark::NONE,
            Permission::FETCH | Permission::STORE | Permission::VEST,
            None,
        )?;
        let token = pie.token();
        pend::insert(&task, pie)?;
        Ok(token.get())
    })();
    answer(frame, r);
}

#[cfg(debug_assertions)]
static FAIL_POLE: ::core::sync::atomic::AtomicUsize = ::core::sync::atomic::AtomicUsize::new(0);

#[cfg(debug_assertions)]
pub(crate) fn fail_pole_at(stage: usize) {
    assert!(crate::testing());
    FAIL_POLE.store(stage, ::core::sync::atomic::Ordering::Relaxed);
}

fn pole_checkpoint(_stage: usize) -> Result<(), PieFail> {
    #[cfg(debug_assertions)]
    if FAIL_POLE
        .compare_exchange(
            _stage,
            0,
            ::core::sync::atomic::Ordering::Relaxed,
            ::core::sync::atomic::Ordering::Relaxed,
        )
        .is_ok()
    {
        return Err(PieFail::OoM);
    }
    Ok(())
}

pub(crate) fn create_pole(
    task: &Arc<crate::core::work::unit::task::Task>,
    reference: FlatReference,
    shared: bool,
) -> Result<PieToken, PieFail> {
    let _serial = task.pend.lock();
    if !pend::live(task) {
        return Err(PieFail::Dead);
    }
    let hold = task
        .pend
        .references
        .get(reference.get())
        .ok_or(PieFail::Denied)?;
    let authorization = hold.source().clone();
    // 只能从**私有根**铸新根：子 Pie / Pole 派生 / 借用都不产生新根资格。
    let source = authorization.own_range().ok_or(PieFail::Denied)?;
    let (offset, count) = hold.window();
    if !authorization.private_owner(task.ident.id)
        || hold.retains()
        || hold.branch().is_some()
        || offset.get() != 0
        || count != source.page_count()
    {
        return Err(PieFail::Denied);
    }
    // 一次性创建资格：单次 claim，竞争方 Busy、已消费 Denied。
    authorization.claim()?;
    let result = (|| {
        pole_checkpoint(1)?;
        let size = source
            .page_count()
            .checked_bytes()
            .ok_or(PieFail::NotAligned)?;
        let meta = mail::pole::PoleMeta::from_source(source.clone(), size, task.ident.id)?;
        let mut permission = Permission::FETCH | Permission::STORE | Permission::VEST;
        if !shared {
            permission |= Permission::ONLY;
        }
        let pie = pend::Pie::try_new(Mail::Pole(meta.clone()), Mark::NONE, permission, None)?;
        let token = pie.token();
        let mut inputs = Vec::new();
        for space in authorization.spaces()? {
            let entries = space.views.owned_by(&authorization)?;
            inputs
                .try_reserve(entries.len())
                .map_err(|_| PieFail::OoM)?;
            for (_, entry) in entries {
                if let Some(view) = entry.view() {
                    inputs.push((space.clone(), view.clone()));
                }
            }
        }
        meta.reserve_records(inputs.len())?;
        task.pend
            .pies
            .lock()
            .try_reserve(1)
            .map_err(|_| PieFail::OoM)?;
        pole_checkpoint(2)?;

        let _commit = crate::core::work::commit();
        let _publication = authorization.begin_withdraw()?;
        if !pend::live(task) || !authorization.private_owner(task.ident.id) {
            return Err(PieFail::Denied);
        }
        pole_checkpoint(3)?;
        meta.attach_source(&authorization);
        authorization.adopt(&meta, token, pie.grant().clone())?;
        let transition = (|| {
            pole_checkpoint(4)?;
            for (space, view) in inputs {
                meta.adopt_input(token, &space, view);
            }
            pole_checkpoint(5)?;
            let mut pies = task.pend.pies.lock();
            assert!(pies.len() < pies.capacity(), "reserved root Pie slot");
            meta.commit_source();
            authorization.consume();
            pies.push(pie);
            pend::changed(task);
            Ok(token)
        })();
        if transition.is_err() {
            authorization.restore_private(source.clone());
        }
        transition
    })();
    if result.is_err() {
        authorization.release_claim();
    }
    result
}

fn unseal_pole(frame: &mut TrapContext, reference: FlatReference, shared: bool) {
    let result = current()
        .running_task()
        .ok_or(PieFail::Denied)
        .and_then(|task| create_pole(&task, reference, shared).map(|token| token.get()));
    answer(frame, result);
}

fn open(frame: &mut TrapContext, ident: Arc<TaskIdent>, token: PieToken) {
    let looked = current()
        .running_task()
        .ok_or(PieFail::Denied)
        .and_then(|task| pend::accede(&task, token, Need::Fetch));
    let r = (|| {
        let pie = looked?;
        usable(&pie)?;
        let p = pie.mail().pole().ok_or(PieFail::Denied)?;
        let access = subset_to_access(pie.permission())?;
        mail::pole::open(&p, token, &ident.team.space, access)
    })();
    answer_pair(frame, r);
}

fn shut(frame: &mut TrapContext, ident: Arc<TaskIdent>, token: PieToken) {
    let _ = &ident;
    let r = (|| -> Result<usize, PieFail> {
        let task = current().running_task().ok_or(PieFail::Denied)?;
        let pie = pend::locate(&task, token).ok_or(PieFail::Denied)?;
        if !pie.allows(Need::Fetch) {
            return Err(PieFail::Denied);
        }
        usable(&pie)?;
        let p = pie.mail().pole().ok_or(PieFail::Denied)?;
        mail::pole::shut(&p, token).map(|()| 0)
    })();
    answer(frame, r);
}

/// 使一枚 Pie 失活，连带关掉它名下已落地的关系。
///
/// 核验与 `mail.seal()` 必须在**同一个 pend** 里完成：与这次 Seal 抢同一份
/// 来源的迁移、撤销都在这把 pend 下串行，否则迁移（已核过授权）会在 Seal 的
/// 核验与失活之间把承载交出去。失活之后就放 pend——`close_maps` 要拆页、
/// 要 shootdown，不能带着 pend 做。
///
/// 关图失败（同步未确认 ⇒ `Busy`）时失活不回滚：这一枚还欠着关图，资源本身
/// 已失活时**原属主**可以对同一枚仍在表里的 token 重发 Seal 把关闭补完；资源
/// 仍活而这一枚的 Grant 已不答话，或换别的任务来，都照旧拒。
fn seal(frame: &mut TrapContext, token: PieToken) {
    let r = (|| -> Result<usize, PieFail> {
        let me = current().running_task().ok_or(PieFail::Denied)?;
        let (pole, _operation) = {
            let _serial = me.pend.lock();
            let pie = pend::locate(&me, token).ok_or(PieFail::Denied)?;
            let pole = pie.mail().pole();
            if !pie.alive() && !pole.as_ref().is_some_and(|meta| meta.has_maps()) {
                return Err(PieFail::Dead);
            }
            // 补关资格只给“来源本身已失活、还欠着关图”那一档，且按 Mail 自己记的
            // 属主判：`Pie::owner()` 是 `alive().then(…)`，失活之后恒为 `None`，不给
            // 这条例外就没有人能把上面的图接着关完。来源仍活那一档照旧按这一枚的
            // 授权事实判——Grant 已不答话是授权失效，不是补关。两档都不恢复活权限。
            let cleanup = !pie.mail().alive() && pie.mail().owner() == me.ident.id;
            if pie.owner() != Some(me.ident.id) && !cleanup {
                return Err(PieFail::Denied);
            }
            let operation = if let Some(p) = &pole {
                if p.reserved() != 0 {
                    return Err(PieFail::Busy);
                }
                Some(PoleOperation::acquire(p).ok_or(PieFail::Busy)?)
            } else {
                None
            };
            pie.mail().seal();
            (pole, operation)
        };
        if let Some(pole) = pole {
            pole.close_maps(None)?;
        }
        Ok(0)
    })();
    answer(frame, r);
}

fn accord(
    frame: &mut TrapContext,
    src_token: PieToken,
    dst_id: TaskId,
    subset: Permission,
    mark: Mark,
) {
    let r = (|| -> Result<usize, PieFail> {
        let caller = current().running_task().ok_or(PieFail::Denied)?;
        let src = pend::locate(&caller, src_token).ok_or(PieFail::Denied)?;
        usable(&src)?;
        let dst = muster(dst_id).ok_or(PieFail::Denied)?;
        pend::accord(&caller, src_token, dst.as_weak(), subset, mark)
    })();
    answer(frame, r);
}

fn narrow(frame: &mut TrapContext, token: PieToken, subset: Permission) {
    let r = current()
        .running_task()
        .ok_or(PieFail::Denied)
        .and_then(|task| pend::reduce(&task, token, subset))
        .map(|()| 0);
    answer(frame, r);
}

fn revoke(frame: &mut TrapContext, dst_id: TaskId, token: PieToken) {
    let r = (|| -> Result<usize, PieFail> {
        let caller = current().running_task().ok_or(PieFail::Denied)?;
        let target = muster(dst_id).ok_or(PieFail::Denied)?;
        pend::revoke(&caller, target.as_weak(), token).map(|_| 0)
    })();
    answer(frame, r);
}

fn information(task: &Arc<crate::core::work::unit::task::Task>, pie: &PieSnapshot) -> PieInfo {
    PieInfo {
        token: pie.token(),
        kind: pie.mail().kind(),
        permission: pie.permission(),
        owner: pie.mail().owner(),
        vestor: if pie.sire().is_some() {
            pend::vestor(task, pie.token()).unwrap_or(TaskId::new(0))
        } else {
            TaskId::new(0)
        },
        mark: pie.mark(),
        alive: pie.alive(),
    }
}

fn write_info(ident: &TaskIdent, buf: usize, info: PieInfo) -> Result<(), PieFail> {
    let words = info.words();
    let bytes =
        unsafe { ::core::slice::from_raw_parts(words.as_ptr().cast::<u8>(), PieInfo::SIZE) };
    if !ident.team.space.copy_out(bytes, KVirt::wrap(buf)) {
        return Err(PieFail::Denied);
    }
    Ok(())
}

fn inspect(frame: &mut TrapContext, ident: &TaskIdent, token: PieToken, buf: VirtAddr) {
    let r = (|| -> Result<usize, PieFail> {
        let task = current().running_task().ok_or(PieFail::Denied)?;
        let pie = pend::locate(&task, token).ok_or(PieFail::Denied)?;
        if !ident
            .team
            .space
            .validate_access(KVirt::wrap(buf.get()), PieInfo::SIZE, Access::WRITE)
        {
            return Err(PieFail::Denied);
        }
        write_info(ident, buf.get(), information(&task, &pie))?;
        Ok(0)
    })();
    answer(frame, r);
}

fn collect(
    frame: &mut TrapContext,
    ident: &TaskIdent,
    after: PieToken,
    buf: VirtAddr,
    capacity: usize,
) {
    let r = (|| -> Result<usize, PieFail> {
        let task = current().running_task().ok_or(PieFail::Denied)?;
        let len = capacity.checked_mul(PieInfo::SIZE).ok_or(PieFail::Denied)?;
        if capacity == 0 {
            return Ok(0);
        }
        if !ident
            .team
            .space
            .validate_access(KVirt::wrap(buf.get()), len, Access::WRITE)
        {
            return Err(PieFail::Denied);
        }
        let capacity = capacity.min(16);
        let mut snapshots: [Option<PieSnapshot>; 16] = ::core::array::from_fn(|_| None);
        let mut count = 0;
        {
            let table = task.pend.pies.lock();
            for pie in table.iter().filter(|p| p.token().get() > after.get()) {
                let at = snapshots[..count]
                    .partition_point(|p| p.as_ref().unwrap().token().get() < pie.token().get());
                if at < capacity {
                    let next = (count + 1).min(capacity);
                    for i in (at + 1..next).rev() {
                        snapshots[i] = snapshots[i - 1].take();
                    }
                    snapshots[at] = Some(pie.snapshot());
                    count = next;
                }
            }
        }
        for (i, pie) in snapshots[..count].iter().enumerate() {
            write_info(
                ident,
                buf.get() + i * PieInfo::SIZE,
                information(&task, pie.as_ref().unwrap()),
            )?;
        }
        Ok(count)
    })();
    answer(frame, r);
}

fn release(frame: &mut TrapContext, token: PieToken, mode: ReleaseMode) {
    let r = current()
        .running_task()
        .ok_or(PieFail::Denied)
        .and_then(|task| match mode {
            ReleaseMode::Revoke => pend::release(&task, token).map(|_| ()),
            ReleaseMode::Keep => pend::forget(&task, token),
        })
        .map(|_| 0);
    answer(frame, r);
}

fn unseal_tole(frame: &mut TrapContext, shared: bool) {
    let r = (|| -> Result<usize, PieFail> {
        let task = current().running_task().ok_or(PieFail::Denied)?;
        let meta = mail::tole::ToleMeta::try_new(task.ident.id).map_err(|_| PieFail::OoM)?;
        let mut permission = Permission::FETCH | Permission::STORE | Permission::VEST;
        if !shared {
            permission |= Permission::ONLY;
        }
        let pie = pend::Pie::try_new(Mail::Tole(meta), Mark::NONE, permission, None)?;
        let token = pie.token();
        pend::insert(&task, pie)?;
        Ok(token.get())
    })();
    answer(frame, r);
}
