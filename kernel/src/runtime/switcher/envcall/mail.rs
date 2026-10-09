use alloc::sync::Arc;

use env::{MailCall, MailCondition, MailFail, Oversize, PieToken, PullOutcome, TaskId, Wait};

use riscv::register::sie;

use crate::memory::manager::entry::PteFlags;
use crate::runtime::switcher::context::{Gprs, TrapContext};
use crate::work::mail;
use crate::work::room::messenger::Handoff;
use crate::work::room::scheduler::core::current;
use crate::work::unit::gate::{self, Need, PieSnapshot};
use crate::work::unit::space::Space;

use super::pie::usable;
use crate::work::unit::task::TaskIdent;

pub(crate) enum Outcome {
    Resume,
    Park(*mut TrapContext),
}

pub(crate) fn dispatch(
    frame: &mut TrapContext,
    call: MailCall,
    ident: Arc<TaskIdent>,
) -> Option<Outcome> {
    Some(match call {
        MailCall::Push { token, msg, len } => push(frame, ident, token, msg.get(), len),
        MailCall::Pull {
            token,
            buf,
            max,
            oversize,
        } => pull(frame, ident, token, buf.get(), max, oversize),
        MailCall::Peek { token } => peek(frame, token),
        MailCall::Wait {
            token,
            condition,
            millis,
        } => wait_dir(frame, ident, token, condition, millis),
        MailCall::Hush { token } => hush(frame, token),
        MailCall::Ring { token } => ring(frame, token),
    })
}

fn push(
    frame: &mut TrapContext,
    ident: Arc<TaskIdent>,
    token: PieToken,
    msg: usize,
    len: usize,
) -> Outcome {
    let me = current()
        .running_task()
        .map(|t| t.ident.id)
        .unwrap_or(TaskId::new(0));
    let r = with_pie(token, Need::Store, |pie| {
        let hole = pie.hole().ok_or(MailFail::Denied)?;
        if len == 0 || !mail::whole(&ident.team.space, msg, len, PteFlags::R) {
            return Err(MailFail::Denied);
        }
        let reservation = mail::hole::reserve_len(&hole, len)?;
        let mut cell = alloc::vec::Vec::new();
        cell.try_reserve_exact(len).map_err(|_| MailFail::OoM)?;
        cell.resize(len, 0);
        if !mail::copy_in(&ident.team.space, &mut cell, msg) {
            return Err(MailFail::Denied);
        }
        let bytes = Arc::try_new(cell).map_err(|_| MailFail::OoM)?;
        reservation.commit(bytes, me)
    });
    frame.gpr.set_x(
        Gprs::A0,
        match r {
            Ok(()) => 0,
            Err(e) => e.code() as usize,
        },
    );
    Outcome::Resume
}

fn pull(
    frame: &mut TrapContext,
    ident: Arc<TaskIdent>,
    token: PieToken,
    buf: usize,
    max: usize,
    oversize: Oversize,
) -> Outcome {
    if !Space::user_range(buf, max) {
        frame.gpr.set_x(Gprs::A0, MailFail::Denied.code() as usize);
        return Outcome::Resume;
    }
    let r = with_pie(token, Need::Fetch, |pie| {
        let hole = pie.hole().ok_or(MailFail::Denied)?;
        if !mail::whole(&ident.team.space, buf, max, PteFlags::W) {
            return Err(MailFail::Denied);
        }
        hand_over(&hole, &ident.team.space, buf, max, oversize)
    });
    match r {
        Ok(outcome) => {
            let (n, from, discarded) = match outcome {
                PullOutcome::Received { len, sender } => (len, sender, false),
                PullOutcome::Discarded { len, sender } => (len, sender, true),
            };
            frame.gpr.set_x(Gprs::A0, n);
            frame.gpr.set_x(Gprs::A1, from.get());
            frame.gpr.set_x(Gprs::A2, discarded as usize);
        }
        Err(e) => frame.gpr.set_x(Gprs::A0, e.code() as usize),
    }
    Outcome::Resume
}

/// Claim the head once. Discard only an oversized message; Keep and copy failures restore it.
/// The reading guard retains the bytes while copies happen outside the queue lock.
fn hand_over(
    meta: &Arc<mail::HoleMeta>,
    space: &Arc<Space>,
    buf: usize,
    max: usize,
    oversize: Oversize,
) -> Result<PullOutcome, MailFail> {
    let reading = mail::hole::read(meta)?;
    let from = reading.from;
    let len = reading.bytes.len();
    if len > max && oversize == Oversize::Discard {
        reading.finish();
        return Ok(PullOutcome::Discarded { len, sender: from });
    }
    if len > max || !mail::whole(space, buf, len, PteFlags::W) {
        // **"读不成、手放回"这一条要看得见**（诊断）：孔会因此**一直报就绪**——等在这一组上的
        // 读的人每一轮都啃同一格（读不成 ⇒ 手还在 ⇒ 下一轮又报就绪），而**别的客人的手就被饿在
        // 后面**；递手的那一方还在等它下线（`wait(MailCondition::Empty, …)`）⇒ 两边一起卡住。
        mail::hole::note_back(len, max);
        return Err(MailFail::Denied);
    }
    if !mail::copy_out(space, &reading.bytes, buf) {
        // 收方那段写不进去（同一格的另一个成因）：手放回原处，下一次换够大的缓冲再来。
        mail::hole::note_back(len, max);
        return Err(MailFail::Denied);
    }
    reading.finish();
    Ok(PullOutcome::Received { len, sender: from })
}

/// 只看那只手：`(长度, 发送者, 队里排着几只)`。不动状态、不唤醒、不复制。
///
/// 第三格（A2）此前空着：单槽时代"排着几只"恒为一件事，没有可报的。队列化之后它是写者
/// 唯一的凭据——"我还排着几手"（`hand::Sender` 那一侧靠它把缓冲收回来）。
fn peek(frame: &mut TrapContext, token: PieToken) -> Outcome {
    let r = with_pie(token, Need::Fetch, |pie| {
        let hole = pie.hole().ok_or(MailFail::Denied)?;
        mail::hole::peek(&hole)
    });
    match r {
        Ok((n, from, depth)) => {
            frame.gpr.set_x(Gprs::A0, n);
            frame.gpr.set_x(Gprs::A1, from.get());
            frame.gpr.set_x(Gprs::A2, depth);
        }
        Err(e) => frame.gpr.set_x(Gprs::A0, e.code() as usize),
    }
    Outcome::Resume
}

/// 等待 Pull 可读、Push 有空间或 Empty 已清空；唤醒后调用方仍须复核。
fn wait_dir(
    frame: &mut TrapContext,
    ident: Arc<TaskIdent>,
    token: PieToken,
    dir: MailCondition,
    millis: Wait,
) -> Outcome {
    enum Ready {
        Hole(Arc<mail::hole::HoleMeta>),
        Bell(Arc<mail::nole::NoleMeta>),
        /// **页上那一位**（架把铃并进页；只有 `Pull` 一条方向，与门铃同）。
        Page(Arc<mail::pole::PoleMeta>),
    }
    let need = match dir {
        MailCondition::Pull => Need::Fetch,
        MailCondition::Push => Need::Store,
        MailCondition::Empty => Need::Store,
    };
    let resolved = with_pie(token, need, |pie| {
        if let Some(hole) = pie.hole() {
            return Ok(Ready::Hole(hole));
        }
        if dir == MailCondition::Pull {
            if let Some(nole) = pie.nole() {
                return Ok(Ready::Bell(nole));
            }
            if let Some(pole) = pie.pole() {
                return Ok(Ready::Page(pole));
            }
        }
        Err(MailFail::Denied)
    });
    let dur = millis.into_duration();
    match resolved {
        Err(e) => frame.gpr.set_x(Gprs::A0, e.code() as usize),
        Ok(ready) => {
            frame.gpr.set_x(Gprs::A0, 0);
            drop(ident);
            let parked = match &ready {
                Ready::Hole(meta) => mail::hole::wait(meta, dir, dur),
                Ready::Bell(meta) => mail::nole::wait(meta, dur),
                Ready::Page(meta) => mail::pole::wait(meta, dur),
            };
            match parked {
                Ok(Handoff::Resume(ready)) => frame.gpr.set_x(Gprs::A0, ready as usize),
                Ok(Handoff::Switch(pa)) => return Outcome::Park(pa as *mut TrapContext),
                Err(e) => frame.gpr.set_x(Gprs::A0, e.code() as usize),
            }
        }
    }
    Outcome::Resume
}

fn hush(frame: &mut TrapContext, token: PieToken) -> Outcome {
    let r = with_pie(token, Need::Fetch, |pie| {
        if let Some(nole) = pie.nole() {
            mail::nole::hush(&nole)?;
            // Only the Nole interrupt path reopens the external interrupt gate.
            unsafe {
                sie::set_sext();
            }
            return Ok(());
        }
        if let Some(hole) = pie.hole() {
            return mail::hole::hush(&hole);
        }
        if let Some(pole) = pie.pole() {
            return mail::pole::hush(&pole);
        }
        Err(MailFail::Denied)
    });
    frame.gpr.set_x(
        Gprs::A0,
        match r {
            Ok(()) => 0,
            Err(e) => e.code() as usize,
        },
    );
    Outcome::Resume
}

fn ring(frame: &mut TrapContext, token: PieToken) -> Outcome {
    let r = with_pie(token, Need::Store, |pie| {
        if let Some(nole) = pie.nole() {
            return mail::nole::ring(&nole);
        }
        if let Some(hole) = pie.hole() {
            return mail::hole::ring(&hole);
        }
        if let Some(pole) = pie.pole() {
            return mail::pole::ring(&pole);
        }
        Err(MailFail::Denied)
    });
    frame.gpr.set_x(
        Gprs::A0,
        match r {
            Ok(()) => 0,
            Err(e) => e.code() as usize,
        },
    );
    Outcome::Resume
}

/// 认一枚门闩、验权、把 `op` 作用上去——门铃与孔上那一位共用这两手。
fn with_pie<T>(
    token: PieToken,
    need: Need,
    op: impl FnOnce(&PieSnapshot) -> Result<T, MailFail>,
) -> Result<T, MailFail> {
    let found = current()
        .running_task()
        .ok_or(MailFail::Denied)
        .and_then(|t| gate::accede::<MailFail>(&t, token, need));
    match found {
        Err(e) => Err(e),
        Ok(pie) => match usable::<MailFail>(&pie) {
            Err(e) => Err(e),
            Ok(()) => op(&pie),
        },
    }
}
