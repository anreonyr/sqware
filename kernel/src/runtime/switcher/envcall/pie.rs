use alloc::sync::Arc;

use env::{
    HoleLimits, Mark, PieCall, PieFail, PieInfo, PieToken, ReleaseMode, TaskId, UnsealArgs,
    VirtAddr,
};

use crate::memory::manager::entry::PteFlags;
use crate::runtime::switcher::context::{Gprs, TrapContext};
use crate::work::mail;
use crate::work::room::messenger::{self, WakeKey};
use crate::work::room::scheduler::core::{current, muster};
use crate::work::unit::gate::{
    self, GateFail, Hole, Need, Nole, Permission, Pie, PieSnapshot, Pole, clear_heir,
};
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
            UnsealArgs::Pole { size, shared } => unseal_pole(frame, size, shared),
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
                    .and_then(|task| gate::same(&task, a, b))
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

pub(super) fn usable<E: GateFail>(pie: &PieSnapshot) -> Result<(), E> {
    if let Some(p) = pie.pole() {
        if p.backing().reserved() != 0 {
            return Err(E::handed_over());
        }
    }
    let Some(h) = pie.heir().copied() else {
        return Ok(());
    };
    let held = muster(h.task)
        .and_then(|t| t.upgrade())
        .is_some_and(|t| t.gate.pies.lock().iter().any(|p| p.token() == h.token));
    if held {
        return Err(E::handed_over());
    }
    if let Some(task) = current().running_task()
        && clear_heir(&task, pie.token(), h)
    {
        // 就地清掉一格陈旧的 heir 也是能力状态变化（它改的是"这一枚还能不能授出"）。
        let _ = messenger::signal(WakeKey::Capabilities {
            task: task.ident.id,
        });
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
        let pie: Pie<Hole> = gate::new_pie(
            meta,
            mark,
            Permission::FETCH | Permission::STORE | Permission::VEST,
            None,
        );
        let token = pie.token;
        gate::insert(&task, gate::boxed(pie)?)?;
        // 本地造一枚：权限表的枚举结果变了。出锁之后要求复核一次
        // （没有观察者时 `signal` 不建站点）。
        let _ = messenger::signal(WakeKey::Capabilities {
            task: task.ident.id,
        });
        Ok(token.get())
    })();
    answer(frame, r);
    Outcome::Resume
}

/// 解封一枚 Nole：**谁都能造**。
///
/// **这里曾有一道 `is_supervisor()` 的门**（`80400b2` 那次带进来的），已撤：一枚 Nole 能换来的
/// 只有"一次唤醒"——它的全部内容是"这一枚存在 ＋ 一位"，没有数据面（见 `mail::nole::NoleMeta`）。
/// **凭证是"谁把它交给你"（`Accord`），不是"谁造的"**：自铸一枚铃不构成提权。同一句话在
/// `env::call::unit::Build` 那一格已经写过一次——那道以 Nole 为凭证的门被撤掉，理由正是
/// "`UnsealNole` 无代价可自铸 ⇒ 那道门与'是 S 态'等价，白收一个载荷"。故这一道一并撤掉，
/// 与 `unseal_hole` / `unseal_pole` 同一口径（那两处本来就没有特权级门）。
fn unseal_nole(frame: &mut TrapContext) -> Outcome {
    let r = (|| -> Result<usize, PieFail> {
        let task = current().running_task().ok_or(PieFail::Denied)?;
        let meta = mail::nole::NoleMeta::try_new(task.ident.id).map_err(|_| PieFail::OoM)?;
        let pie: Pie<Nole> = gate::new_pie(
            meta,
            Mark::NONE,
            Permission::FETCH | Permission::STORE | Permission::VEST,
            None,
        );
        let token = pie.token;
        gate::insert(&task, gate::boxed(pie)?)?;
        // 本地造一枚：权限表的枚举结果变了。
        let _ = messenger::signal(WakeKey::Capabilities {
            task: task.ident.id,
        });
        Ok(token.get())
    })();
    answer(frame, r);
    Outcome::Resume
}

fn unseal_pole(frame: &mut TrapContext, size: usize, shared: bool) -> Outcome {
    let r = (|| -> Result<usize, PieFail> {
        let task = current().running_task().ok_or(PieFail::Denied)?;
        let meta = mail::pole::meta(size, task.ident.id)?;
        let task_space = task.ident.team.space.clone();
        let mut permission = Permission::FETCH | Permission::STORE | Permission::VEST;
        if !shared {
            permission |= Permission::ONLY;
        }
        let pie: Pie<Pole> = gate::try_new_pie(meta.clone(), Mark::NONE, permission, None)?;
        let token = pie.token;
        let creator_flags = task_space
            .pte_policy(PteFlags::V | PteFlags::R | PteFlags::W | PteFlags::A | PteFlags::D);
        mail::pole::open(&meta, token, &task_space, creator_flags)?;
        if let Err(error) = gate::boxed(pie).and_then(|pie| gate::insert(&task, pie)) {
            mail::pole::shut(&meta, token)?;
            return Err(error);
        }
        // 本地造一枚：权限表的枚举结果变了。
        let _ = messenger::signal(WakeKey::Capabilities {
            task: task.ident.id,
        });
        Ok(token.get())
    })();
    answer(frame, r);
    Outcome::Resume
}

fn open(frame: &mut TrapContext, ident: Arc<TaskIdent>, token: PieToken) -> Outcome {
    let looked = current()
        .running_task()
        .ok_or(PieFail::Denied)
        .and_then(|task| gate::accede::<PieFail>(&task, token, Need::Fetch));
    let r = (|| {
        let pie = looked?;
        usable::<PieFail>(&pie)?;
        let p = pie.pole().ok_or(PieFail::Denied)?;
        let _operation = p.backing().operation().ok_or(PieFail::Busy)?;
        if p.backing().reserved() != 0 {
            return Err(PieFail::HandedOver);
        }
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
        let pie = gate::locate(&task, token).ok_or(PieFail::Denied)?;
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
        let pie = gate::locate(&me, token).ok_or(PieFail::Denied)?;
        if !pie.alive() {
            return Err(PieFail::Dead);
        }
        if pie.owner() != Some(me.ident.id) {
            return Err(PieFail::Denied);
        }
        let pole = pie.pole();
        let _operation = if let Some(p) = &pole {
            let operation = p.backing().operation().ok_or(PieFail::Busy)?;
            if p.backing().reserved() != 0 {
                return Err(PieFail::Busy);
            }
            Some(operation)
        } else {
            None
        };
        pie.seal();
        // 资源封印让这一枚能力失效——表项还在，所以枚举结果本身就变了。
        let _ = messenger::signal(WakeKey::Capabilities { task: me.ident.id });
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
        let src = gate::locate(&caller, src_token).ok_or(PieFail::Denied)?;
        usable::<PieFail>(&src)?;
        let dst = muster(dst_id).ok_or(PieFail::Denied)?;
        gate::accord(&caller, src_token, &dst, subset, mark)
    })();
    answer(frame, r);
    Outcome::Resume
}

fn narrow(frame: &mut TrapContext, token: PieToken, subset: Permission) -> Outcome {
    let r = current()
        .running_task()
        .ok_or(PieFail::Denied)
        .and_then(|task| gate::reduce(&task, token, subset))
        .map(|()| 0);
    answer(frame, r);
    Outcome::Resume
}

fn revoke(frame: &mut TrapContext, dst_id: TaskId, token: PieToken) -> Outcome {
    let r = (|| -> Result<usize, PieFail> {
        let caller = current().running_task().ok_or(PieFail::Denied)?;
        let target = muster(dst_id).ok_or(PieFail::Denied)?;
        gate::revoke(&caller, &target, token).map(|_| 0)
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
            gate::vestor(task, pie.token()).unwrap_or(TaskId::new(0))
        } else {
            TaskId::new(0)
        },
        mark: pie.mark(),
        alive: pie.alive(),
    }
}

fn write_info(ident: &TaskIdent, buf: usize, info: PieInfo) -> Result<(), PieFail> {
    let words = info.words();
    // Every word is initialized; only its byte representation crosses the ABI.
    let bytes = unsafe { core::slice::from_raw_parts(words.as_ptr().cast::<u8>(), PieInfo::SIZE) };
    if !mail::copy_out(&ident.team.space, bytes, buf) {
        return Err(PieFail::Denied);
    }
    Ok(())
}

fn inspect(frame: &mut TrapContext, ident: &TaskIdent, token: PieToken, buf: VirtAddr) -> Outcome {
    let r = (|| {
        let task = current().running_task().ok_or(PieFail::Denied)?;
        let pie = gate::locate(&task, token).ok_or(PieFail::Denied)?;
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
        // A bounded stack batch avoids heap allocation and releases the table lock before copying.
        let capacity = capacity.min(16);
        let mut snapshots: [Option<PieSnapshot>; 16] = core::array::from_fn(|_| None);
        let mut count = 0;
        {
            let table = task.gate.pies.lock();
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
            ReleaseMode::Revoke => gate::release(&task, token).map(|_| ()),
            ReleaseMode::Keep => gate::forget(&task, token),
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
        let pie: Pie<gate::Tole> = gate::new_pie(meta, Mark::NONE, permission, None);
        let token = pie.token;
        gate::insert(&task, gate::boxed(pie)?)?;
        let _ = messenger::signal(WakeKey::Capabilities {
            task: task.ident.id,
        });
        Ok(token.get())
    })();
    answer(frame, r);
    Outcome::Resume
}
