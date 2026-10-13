use alloc::sync::Arc;

use env::{
    HoleLimits, Mark, PieCall, PieFail, PieInfo, PieToken, ReleaseMode, TaskId, UnsealArgs,
    VirtAddr,
};

use crate::memory::manager::entry::PteFlags;
use crate::runtime::switcher::context::{Gprs, TrapContext};
use crate::work::mail;
use crate::work::pend::{
    self, Hole, Need, Nole, PendFail, Permission, Pie, PieSnapshot, Pole, clear_heir,
};
use crate::work::room::scheduler::core::{current, muster};
use crate::work::unit::task::TaskIdent;

fn subset_to_pte(subset: Permission) -> Result<PteFlags, PieFail> {
    if !subset.contains(Permission::FETCH) {
        return Err(PieFail::Denied);
    }
    let mut f = PteFlags::V | PteFlags::A | PteFlags::D;
    f |= PteFlags::R;
    if subset.contains(Permission::STORE) {
        f |= PteFlags::W;
    }
    Ok(f)
}

pub(crate) enum Outcome {
    Resume,
}

pub(crate) fn dispatch(
    frame: &mut TrapContext,
    call: PieCall,
    ident: Arc<TaskIdent>,
) -> Option<Outcome> {
    let _ = &ident;
    Some(match call {
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
            Outcome::Resume
        }
    })
}

fn answer(frame: &mut TrapContext, r: Result<usize, PieFail>) {
    frame.gpr.set_x(
        Gprs::A0,
        match r {
            Ok(v) => v,
            Err(e) => e.code() as usize,
        },
    );
}

fn answer_pair(frame: &mut TrapContext, r: Result<(usize, usize), PieFail>) {
    match r {
        Ok((v0, v1)) => {
            frame.gpr.set_x(Gprs::A0, v0);
            frame.gpr.set_x(Gprs::A1, v1);
        }
        Err(e) => frame.gpr.set_x(Gprs::A0, e.code() as usize),
    }
}

pub(crate) fn usable<E: PendFail>(pie: &PieSnapshot) -> Result<(), E> {
    if let Some(p) = pie.pole() {
        if p.reserved() != 0 {
            return Err(E::handed_over());
        }
    }
    let Some(h) = pie.heir().copied() else {
        return Ok(());
    };
    let held = muster(h.task)
        .and_then(|t| t.upgrade())
        .is_some_and(|t| t.pend.pies.lock().iter().any(|p| p.token() == h.token));
    if held {
        return Err(E::handed_over());
    }
    if let Some(task) = current().running_task() {
        clear_heir(&task, pie.token(), h);
    }
    Ok(())
}

fn unseal_hole(frame: &mut TrapContext, mark: Mark, limits: HoleLimits) -> Outcome {
    let r = (|| -> Result<usize, PieFail> {
        let task = current().running_task().ok_or(PieFail::Denied)?;
        if !limits.valid() {
            return Err(PieFail::Denied);
        }
        let meta =
            mail::hole::try_meta_with_limits(task.ident.id, limits).map_err(|_| PieFail::OoM)?;
        let pie: Pie<Hole> = pend::new_pie(
            meta,
            mark,
            Permission::FETCH | Permission::STORE | Permission::VEST,
            None,
        );
        let token = pie.token;
        pend::insert(&task, pend::boxed(pie)?)?;
        Ok(token.get())
    })();
    answer(frame, r);
    Outcome::Resume
}

fn unseal_nole(frame: &mut TrapContext) -> Outcome {
    let r = (|| -> Result<usize, PieFail> {
        let task = current().running_task().ok_or(PieFail::Denied)?;
        let meta = mail::nole::NoleMeta::try_new(task.ident.id).map_err(|_| PieFail::OoM)?;
        let pie: Pie<Nole> = pend::new_pie(
            meta,
            Mark::NONE,
            Permission::FETCH | Permission::STORE | Permission::VEST,
            None,
        );
        let token = pie.token;
        pend::insert(&task, pend::boxed(pie)?)?;
        Ok(token.get())
    })();
    answer(frame, r);
    Outcome::Resume
}

#[cfg(debug_assertions)]
static FAIL_POLE: core::sync::atomic::AtomicUsize = core::sync::atomic::AtomicUsize::new(0);

#[cfg(debug_assertions)]
pub(crate) fn fail_pole_at(stage: usize) {
    assert!(crate::testing());
    FAIL_POLE.store(stage, core::sync::atomic::Ordering::Relaxed);
}

fn pole_checkpoint(_stage: usize) -> Result<(), PieFail> {
    #[cfg(debug_assertions)]
    if FAIL_POLE
        .compare_exchange(
            _stage,
            0,
            core::sync::atomic::Ordering::Relaxed,
            core::sync::atomic::Ordering::Relaxed,
        )
        .is_ok()
    {
        return Err(PieFail::OoM);
    }
    Ok(())
}

pub(crate) fn create_pole(
    task: &Arc<crate::work::unit::task::Task>,
    addr: crate::memory::manager::addr::VirtAddr,
    size: usize,
    shared: bool,
) -> Result<PieToken, PieFail> {
    let task_space = task.ident.team.space.clone();
    let mapping = task_space
        .mapping_at(addr, size)
        .map_err(|_| PieFail::Denied)?;
    let creation = task.pend.creations.claim(&mapping.map)?;
    pole_checkpoint(1)?;
    let meta = mail::pole::PoleMeta::from_mapping(&task_space, mapping, task.ident.id)?;
    pole_checkpoint(2)?;
    let mut permission = Permission::FETCH | Permission::STORE | Permission::VEST;
    if !shared {
        permission |= Permission::ONLY;
    }
    let pie: Pie<Pole> = pend::try_new_pie(meta.clone(), Mark::NONE, permission, None)?;
    let token = pie.token;
    let pie = pend::boxed(pie)?;
    pole_checkpoint(3)?;
    pend::insert(task, pie)?;
    meta.commit_source();
    creation.commit();
    Ok(token)
}

fn unseal_pole(frame: &mut TrapContext, addr: VirtAddr, size: usize, shared: bool) -> Outcome {
    let result = current()
        .running_task()
        .ok_or(PieFail::Denied)
        .and_then(|task| {
            create_pole(
                &task,
                crate::memory::manager::addr::VirtAddr::wrap(addr.get()),
                size,
                shared,
            )
            .map(|token| token.get())
        });
    answer(frame, result);
    Outcome::Resume
}

fn open(frame: &mut TrapContext, ident: Arc<TaskIdent>, token: PieToken) -> Outcome {
    let looked = current()
        .running_task()
        .ok_or(PieFail::Denied)
        .and_then(|task| pend::accede::<PieFail>(&task, token, Need::Fetch));
    let r = (|| {
        let pie = looked?;
        usable::<PieFail>(&pie)?;
        let p = pie.pole().ok_or(PieFail::Denied)?;
        let flags = subset_to_pte(pie.permission())?;
        mail::pole::open(
            &p,
            token,
            &ident.team.space,
            ident.team.space.pte_policy(flags),
        )
    })();
    answer_pair(frame, r);
    Outcome::Resume
}

fn shut(frame: &mut TrapContext, ident: Arc<TaskIdent>, token: PieToken) -> Outcome {
    let _ = &ident;
    let r = (|| -> Result<usize, PieFail> {
        let task = current().running_task().ok_or(PieFail::Denied)?;
        let pie = pend::locate(&task, token).ok_or(PieFail::Denied)?;
        if !pie.allows(Need::Fetch) {
            return Err(PieFail::Denied);
        }
        usable::<PieFail>(&pie)?;
        let p = pie.pole().ok_or(PieFail::Denied)?;
        mail::pole::shut(&p, token).map(|()| 0)
    })();
    answer(frame, r);
    Outcome::Resume
}

fn seal(frame: &mut TrapContext, token: PieToken) -> Outcome {
    let r = (|| -> Result<usize, PieFail> {
        let me = current().running_task().ok_or(PieFail::Denied)?;
        let pie = pend::locate(&me, token).ok_or(PieFail::Denied)?;
        if !pie.alive() {
            return Err(PieFail::Dead);
        }
        if pie.owner() != Some(me.ident.id) {
            return Err(PieFail::Denied);
        }
        let pole = pie.pole();
        let _operation = if let Some(p) = &pole {
            Some(p.operation().ok_or(PieFail::Busy)?)
        } else {
            None
        };
        pie.seal();
        Ok(0)
    })();
    answer(frame, r);
    Outcome::Resume
}

fn accord(
    frame: &mut TrapContext,
    src_token: PieToken,
    dst_id: TaskId,
    subset: Permission,
    mark: Mark,
) -> Outcome {
    let r = (|| -> Result<usize, PieFail> {
        let caller = current().running_task().ok_or(PieFail::Denied)?;
        let src = pend::locate(&caller, src_token).ok_or(PieFail::Denied)?;
        usable::<PieFail>(&src)?;
        let dst = muster(dst_id).ok_or(PieFail::Denied)?;
        pend::accord(&caller, src_token, &dst, subset, mark)
    })();
    answer(frame, r);
    Outcome::Resume
}

fn narrow(frame: &mut TrapContext, token: PieToken, subset: Permission) -> Outcome {
    let r = current()
        .running_task()
        .ok_or(PieFail::Denied)
        .and_then(|task| pend::reduce(&task, token, subset))
        .map(|()| 0);
    answer(frame, r);
    Outcome::Resume
}

fn revoke(frame: &mut TrapContext, dst_id: TaskId, token: PieToken) -> Outcome {
    let r = (|| -> Result<usize, PieFail> {
        let caller = current().running_task().ok_or(PieFail::Denied)?;
        let target = muster(dst_id).ok_or(PieFail::Denied)?;
        pend::revoke(&caller, &target, token).map(|_| 0)
    })();
    answer(frame, r);
    Outcome::Resume
}

fn information(task: &Arc<crate::work::unit::task::Task>, pie: &PieSnapshot) -> PieInfo {
    PieInfo {
        token: pie.token(),
        kind: pie.kind(),
        permission: pie.permission(),
        owner: pie.owner_task(),
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
    let bytes = unsafe { core::slice::from_raw_parts(words.as_ptr().cast::<u8>(), PieInfo::SIZE) };
    if !mail::copy_out(&ident.team.space, bytes, buf) {
        return Err(PieFail::Denied);
    }
    Ok(())
}

fn inspect(frame: &mut TrapContext, ident: &TaskIdent, token: PieToken, buf: VirtAddr) -> Outcome {
    let r = (|| {
        let task = current().running_task().ok_or(PieFail::Denied)?;
        let pie = pend::locate(&task, token).ok_or(PieFail::Denied)?;
        if !mail::whole(&ident.team.space, buf.get(), PieInfo::SIZE, PteFlags::W) {
            return Err(PieFail::Denied);
        }
        write_info(ident, buf.get(), information(&task, &pie))?;
        Ok(0)
    })();
    answer(frame, r);
    Outcome::Resume
}

fn collect(
    frame: &mut TrapContext,
    ident: &TaskIdent,
    after: PieToken,
    buf: VirtAddr,
    capacity: usize,
) -> Outcome {
    let r = (|| {
        let task = current().running_task().ok_or(PieFail::Denied)?;
        let len = capacity.checked_mul(PieInfo::SIZE).ok_or(PieFail::Denied)?;
        if capacity == 0 {
            return Ok(0);
        }
        if !mail::whole(&ident.team.space, buf.get(), len, PteFlags::W) {
            return Err(PieFail::Denied);
        }
        let capacity = capacity.min(16);
        let mut snapshots: [Option<PieSnapshot>; 16] = core::array::from_fn(|_| None);
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
    Outcome::Resume
}

fn release(frame: &mut TrapContext, token: PieToken, mode: ReleaseMode) -> Outcome {
    let r = current()
        .running_task()
        .ok_or(PieFail::Denied)
        .and_then(|task| match mode {
            ReleaseMode::Revoke => pend::release(&task, token).map(|_| ()),
            ReleaseMode::Keep => pend::forget(&task, token),
        })
        .map(|_| 0);
    answer(frame, r);
    Outcome::Resume
}

fn unseal_tole(frame: &mut TrapContext, shared: bool) -> Outcome {
    let r = (|| {
        let task = current().running_task().ok_or(PieFail::Denied)?;
        let meta = mail::tole::try_meta(task.ident.id).map_err(|_| PieFail::OoM)?;
        let mut permission = Permission::FETCH | Permission::STORE | Permission::VEST;
        if !shared {
            permission |= Permission::ONLY;
        }
        let pie: Pie<pend::Tole> = pend::new_pie(meta, Mark::NONE, permission, None);
        let token = pie.token;
        pend::insert(&task, pend::boxed(pie)?)?;
        Ok(token.get())
    })();
    answer(frame, r);
    Outcome::Resume
}

pub(crate) fn observe(pie: &PieSnapshot) -> Result<(), env::MailFail> {
    if let Some(pole) = pie.pole()
        && pole.reserved() != 0
    {
        return Err(env::MailFail::HandedOver);
    }
    if let Some(heir) = pie.heir()
        && muster(heir.task)
            .and_then(|t| t.upgrade())
            .is_some_and(|t| pend::locate(&t, heir.token).is_some())
    {
        return Err(env::MailFail::HandedOver);
    }
    Ok(())
}
