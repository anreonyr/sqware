use alloc::sync::Arc;

use env::{Mark, PieCall, PieFail, PieToken, TaskId};

use crate::memory::manager::entry::PteFlags;
use crate::runtime::switcher::context::{Gprs, TrapContext};
use crate::work::mail;
use crate::work::room::messenger::{self, WakeKey};
use crate::work::room::scheduler::core::{current, muster};
use crate::work::unit::gate::{
    self, AnyPie, GateFail, Hole, Need, Nole, Permission, Pie, Pole, clear_heir,
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
        PieCall::UnsealHole { mark } => unseal_hole(frame, &ident, mark),
        PieCall::UnsealPole { size, shared } => unseal_pole(frame, size, shared),
        PieCall::UnsealNole => unseal_nole(frame),
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
        PieCall::Collect { index } => collect(frame, index),
        PieCall::Reserve { token } => reserve(frame, token, true),
        PieCall::Inspect { token } => reserve(frame, token, false),
        PieCall::Release { token } => release(frame, token),
        PieCall::Alive { token } => alive(frame, token),
        PieCall::Forget { token } => {
            answer(frame, current().running_task().ok_or(PieFail::Denied)
                .and_then(|task| gate::forget(&task, token)).map(|_| 0));
            Outcome::Resume
        }
        PieCall::Same { a, b } => {
            answer(frame, current().running_task().ok_or(PieFail::Denied)
                .and_then(|task| gate::same(&task, a, b)).map(usize::from));
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

pub(super) fn usable<E: GateFail>(pie: &AnyPie) -> Result<(), E> {
    if let AnyPie::Pole(p) = pie {
        if p.meta().backing().reserved() != 0 { return Err(E::handed_over()); }
    }
    let Some(h) = pie.heir().copied() else {
        return Ok(());
    };
    let held = muster(h.task)
        .and_then(|t| t.upgrade())
        .is_some_and(|t| t.pies.lock().iter().any(|p| p.token() == h.token));
    if held {
        return Err(E::handed_over());
    }
    if let Some(task) = current().running_task()
        && clear_heir(&task, pie.token())
    {
        // 就地清掉一格陈旧的 heir 也是能力状态变化（它改的是"这一枚还能不能授出"）。
        let _ = messenger::signal(WakeKey::Capabilities { task: task.ident.id });
    }
    Ok(())
}

fn unseal_hole(frame: &mut TrapContext, _ident: &TaskIdent, mark: Mark) -> Outcome {
    let r = (|| -> Result<usize, PieFail> {
        let task = current().running_task().ok_or(PieFail::Denied)?;
        let meta = mail::hole::meta(task.ident.id);
        let pie: Pie<Hole> = gate::new_pie(
            meta,
            mark,
            Permission::FETCH | Permission::STORE | Permission::VEST,
            None,
        );
        let token = pie.token;
        {
            let mut pies = task.pies.lock();
            pies.try_reserve(1).map_err(|_| PieFail::OoM)?;
            pies.push(AnyPie::Hole(pie));
        }
        // 本地造一枚：权限表的枚举结果变了。出锁之后要求复核一次
        // （没有观察者时 `signal` 不建站点）。
        let _ = messenger::signal(WakeKey::Capabilities { task: task.ident.id });
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
        let meta = mail::nole::NoleMeta::new(task.ident.id);
        let pie: Pie<Nole> = gate::new_pie(
            meta,
            Mark::NONE,
            Permission::FETCH | Permission::STORE | Permission::VEST,
            None,
        );
        let token = pie.token;
        {
            let mut pies = task.pies.lock();
            pies.try_reserve(1).map_err(|_| PieFail::OoM)?;
            pies.push(AnyPie::Nole(pie));
        }
        // 本地造一枚：权限表的枚举结果变了。
        let _ = messenger::signal(WakeKey::Capabilities { task: task.ident.id });
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
        if !shared { permission |= Permission::ONLY; }
        let pie: Pie<Pole> = gate::try_new_pie(
            meta.clone(),
            Mark::NONE,
            permission,
            None,
        )?;
        let token = pie.token;
        task.pies.lock().try_reserve(1).map_err(|_| PieFail::OoM)?;
        let creator_flags = task_space
            .pte_policy(PteFlags::V | PteFlags::R | PteFlags::W | PteFlags::A | PteFlags::D);
        mail::pole::open(&meta, token, &task_space, creator_flags)?;
        task.pies.lock().push(AnyPie::Pole(pie));
        // 本地造一枚：权限表的枚举结果变了。
        let _ = messenger::signal(WakeKey::Capabilities { task: task.ident.id });
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
    let r = match looked.and_then(|p| usable::<PieFail>(&p).map(|()| p)) {
        Err(e) => Err(e),
        Ok(AnyPie::Pole(p)) => {
            let r = (|| {
                let _operation = p.meta().backing().operation().ok_or(PieFail::Busy)?;
                if p.meta().backing().reserved() != 0 { return Err(PieFail::HandedOver); }
                let flags = subset_to_pte(p.permission())?;
                mail::pole::open(p.meta(), token, &ident.team.space, ident.team.space.pte_policy(flags))
            })();
            r
        },
        Ok(AnyPie::Hole(_)) => Err(PieFail::Denied),
        Ok(AnyPie::Nole(_)) => Err(PieFail::Denied),
        Ok(AnyPie::Tole(_)) => Err(PieFail::Denied),
    };
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
        match pie {
            AnyPie::Pole(p) => mail::pole::shut(p.meta(), token).map(|()| 0),
            AnyPie::Hole(_) | AnyPie::Nole(_) | AnyPie::Tole(_) => Err(PieFail::Denied),
        }
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
        match &pie {
            AnyPie::Hole(h) => mail::hole::seal(h.meta()),
            AnyPie::Pole(pl) => {
                let _operation = pl.meta().backing().operation().ok_or(PieFail::Busy)?;
                if pl.meta().backing().reserved() != 0 { return Err(PieFail::Busy); }
                mail::pole::seal(pl.meta());
            },
            AnyPie::Nole(v) => mail::nole::seal(v.meta()),
            AnyPie::Tole(t) => mail::tole::seal(t.meta()),
        }
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
    let r = current().running_task().ok_or(PieFail::Denied)
        .and_then(|task| gate::reduce(&task, token, subset)).map(|()| 0);
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

fn collect(frame: &mut TrapContext, index: usize) -> Outcome {
    let pie = current()
        .running_task()
        .and_then(|t| t.pies.lock().get(index).cloned());
    let (token, owner_id, mark) = match &pie {
        Some(p) => {
            let owner = match p {
                AnyPie::Hole(_) => p.owner().unwrap_or(TaskId::new(0)),
                _ => TaskId::new(0),
            };
            (p.token(), owner, p.mark())
        }
        None => (PieToken::NONE, TaskId::new(0), Mark::NONE),
    };
    frame.gpr.set_x(Gprs::A0, token.get());
    frame.gpr.set_x(Gprs::A1, owner_id.get());
    frame.gpr.set_x(Gprs::A2, mark.get() as usize);
    Outcome::Resume
}

fn reserve(frame: &mut TrapContext, token: PieToken, hole_only: bool) -> Outcome {
    let r = (|| -> Result<(TaskId, TaskId, usize), PieFail> {
        let task = current().running_task().ok_or(PieFail::Denied)?;
        let p = gate::locate(&task, token).ok_or(PieFail::Denied)?;
        let owner = p.owner().ok_or(PieFail::Dead)?;
        if hole_only && !matches!(p, AnyPie::Hole(_)) {
            return Err(PieFail::Denied);
        }
        let mark = p.mark().get() as usize;
        Ok((
            gate::vestor(&p, &gate::snap()).unwrap_or(TaskId::new(0)),
            owner,
            mark,
        ))
    })();
    match r {
        Ok((vestor_id, owner_id, mark)) => {
            frame.gpr.set_x(
                Gprs::A0,
                (owner_id.get() << 32) | (vestor_id.get() & 0xffff_ffff),
            );
            frame.gpr.set_x(Gprs::A1, mark);
        }
        Err(e) => frame.gpr.set_x(Gprs::A0, e.code() as usize),
    }
    Outcome::Resume
}

fn release(frame: &mut TrapContext, token: PieToken) -> Outcome {
    let r = match current().running_task() {
        Some(task) => gate::release(&task, token).map(|_| 0),
        None => Err(PieFail::Denied),
    };
    answer(frame, r);
    Outcome::Resume
}

/// **这一枚还在不在**（generic：孔 / 页 / 铃 / 组都答得出）。
///
/// 两件一起判：**在我表里**（`gate::locate`）＋ **资源还活着**（`AnyPie::alive`——封印即不在）。
/// 与 `reserve` 的分工写在那两格的注里：那一格答的是孔的来历与记号（故对页与铃答
/// `Denied`），这一格只答存活——树那一层要的正是这一件。
///
/// **别用 `usable` 顶替**：那一手只管"这一枚有没有交出去"（`heir`），**不看资源死活**——
/// 页被封印之后它会照答"在"，树那一格就永远剔不掉（这条是那台探针自己抓出来的）。
/// "交出去了"那一档由 `Accord` 那条路自己答 `HandedOver`，不必在这一格里重复。
///
/// **不失败**：`answer(Ok(0/1))` 恒走成功那一支，故"不在"就是 `false`，不是负码。
fn alive(frame: &mut TrapContext, token: PieToken) -> Outcome {
    let live = current()
        .running_task()
        .and_then(|task| gate::locate(&task, token))
        .is_some_and(|pie| pie.alive());
    answer(frame, Ok(live as usize));
    Outcome::Resume
}
