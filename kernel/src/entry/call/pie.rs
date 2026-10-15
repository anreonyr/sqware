use alloc::sync::Arc;

use env::{
    HoleLimits, Mark, PieCall, PieFail, PieInfo, PieToken, ReleaseMode, TaskId, UnsealArgs,
    VirtAddr,
};

use crate::core::memory::PAGE_SIZE;
use crate::core::memory::address::VirtAddr as KVirt;
use crate::core::memory::nest::{Access, Nest, View};
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
            UnsealArgs::Pole { addr, size, shared } => unseal_pole(frame, addr, size, shared),
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
    addr: crate::core::memory::address::VirtAddr,
    size: usize,
    shared: bool,
) -> Result<PieToken, PieFail> {
    let task_space = task.ident.team.space.clone();
    let view = view_at(&task_space.nest, addr, size).ok_or(PieFail::Denied)?;
    let creation = task.pend.creations.claim(&view)?;
    pole_checkpoint(1)?;
    let meta = mail::pole::PoleMeta::from_mapping(&task_space, view, task.ident.id)?;
    pole_checkpoint(2)?;
    let mut permission = Permission::FETCH | Permission::STORE | Permission::VEST;
    if !shared {
        permission |= Permission::ONLY;
    }
    let pie = pend::Pie::try_new(Mail::Pole(meta.clone()), Mark::NONE, permission, None)?;
    let token = pie.token();
    pole_checkpoint(3)?;
    pend::insert(task, pie)?;
    meta.commit_source();
    creation.commit();
    Ok(token)
}

/// 按线外 VA + 字节跨度取一段已有关系：候选由 VA 给出，身份以 `View` 为准。
pub(in crate::entry) fn view_at(nest: &Nest, start: KVirt, size: usize) -> Option<Arc<View>> {
    if size == 0 || start.offset() != 0 || !size.is_multiple_of(PAGE_SIZE) {
        return None;
    }
    let view = nest.probe(start)?;
    (view.initial_range() == (start, size) && view.is_live()).then_some(view)
}

fn unseal_pole(frame: &mut TrapContext, addr: VirtAddr, size: usize, shared: bool) {
    let result = current()
        .running_task()
        .ok_or(PieFail::Denied)
        .and_then(|task| {
            create_pole(
                &task,
                crate::core::memory::address::VirtAddr::wrap(addr.get()),
                size,
                shared,
            )
            .map(|token| token.get())
        });
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

fn seal(frame: &mut TrapContext, token: PieToken) {
    let r = (|| -> Result<usize, PieFail> {
        let me = current().running_task().ok_or(PieFail::Denied)?;
        let pie = pend::locate(&me, token).ok_or(PieFail::Denied)?;
        if !pie.alive() {
            return Err(PieFail::Dead);
        }
        if pie.owner() != Some(me.ident.id) {
            return Err(PieFail::Denied);
        }
        let pole = pie.mail().pole();
        let _operation = if let Some(p) = &pole {
            Some(PoleOperation::acquire(p).ok_or(PieFail::Busy)?)
        } else {
            None
        };
        pie.mail().seal();
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
