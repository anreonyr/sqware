use alloc::sync::Arc;

use env::{Bit, MailCall, MailCondition, MailFail, Oversize, PieToken, PullOutcome, TaskId, Wait};

use riscv::register::sie;

use crate::memory::manager::entry::PteFlags;
use crate::runtime::switcher::context::{Gprs, TrapContext};
use crate::work::mail;
use crate::work::pend::{self, Need, PieSnapshot};
use crate::work::room::messenger::Handoff;
use crate::work::room::scheduler::core::current;
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
    if matches!(
        call,
        MailCall::Attach { .. } | MailCall::Detach { .. } | MailCall::Await { .. }
    ) {
        return super::tole::dispatch(frame, call, ident);
    }
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
        MailCall::Hush { token, bit } => hush(frame, token, bit),
        MailCall::Ring { token, bit } => ring(frame, token, bit),
        _ => unreachable!(),
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
        let hole = pie.mail().hole().ok_or(MailFail::Denied)?;
        if len == 0 || !ident.team.space.validate_access(msg, len, PteFlags::R) {
            return Err(MailFail::Denied);
        }
        let reservation = mail::hole::reserve_len(&hole, len)?;
        let mut cell = alloc::vec::Vec::new();
        cell.try_reserve_exact(len).map_err(|_| MailFail::OoM)?;
        cell.resize(len, 0);
        if !ident.team.space.copy_in(&mut cell, msg) {
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
        let hole = pie.mail().hole().ok_or(MailFail::Denied)?;
        if !ident.team.space.validate_access(buf, max, PteFlags::W) {
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
    if len > max || !space.validate_access(buf, len, PteFlags::W) {
        mail::hole::note_back(len, max);
        return Err(MailFail::Denied);
    }
    if !space.copy_out(&reading.bytes, buf) {
        mail::hole::note_back(len, max);
        return Err(MailFail::Denied);
    }
    reading.finish();
    Ok(PullOutcome::Received { len, sender: from })
}

fn peek(frame: &mut TrapContext, token: PieToken) -> Outcome {
    let r = with_pie(token, Need::Fetch, |pie| {
        let hole = pie.mail().hole().ok_or(MailFail::Denied)?;
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

fn wait_dir(
    frame: &mut TrapContext,
    ident: Arc<TaskIdent>,
    token: PieToken,
    dir: MailCondition,
    millis: Wait,
) -> Outcome {
    let need = Need::of(dir);
    let dur = millis.into_duration();
    let r = with_pie(token, need, |pie| {
        frame.gpr.set_x(Gprs::A0, 0);
        drop(ident);
        pie.mail().wait(dir, dur)
    });
    match r {
        Ok(Handoff::Resume(ready)) => frame.gpr.set_x(Gprs::A0, ready as usize),
        Ok(Handoff::Switch(pa)) => return Outcome::Park(pa as *mut TrapContext),
        Err(e) => frame.gpr.set_x(Gprs::A0, e.code() as usize),
    }
    Outcome::Resume
}

fn hush(frame: &mut TrapContext, token: PieToken, bit: Bit) -> Outcome {
    let r = with_pie(token, Need::Fetch, |pie| {
        let mail = pie.mail();
        mail.hush(bit)?;
        if mail.kind() == env::PieKind::Nole {
            unsafe {
                sie::set_sext();
            }
        }
        Ok(())
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

fn ring(frame: &mut TrapContext, token: PieToken, bit: Bit) -> Outcome {
    let r = with_pie(token, Need::Store, |pie| pie.mail().ring(bit));
    frame.gpr.set_x(
        Gprs::A0,
        match r {
            Ok(()) => 0,
            Err(e) => e.code() as usize,
        },
    );
    Outcome::Resume
}

fn with_pie<T>(
    token: PieToken,
    need: Need,
    op: impl FnOnce(&PieSnapshot) -> Result<T, MailFail>,
) -> Result<T, MailFail> {
    let found = current()
        .running_task()
        .ok_or(MailFail::Denied)
        .and_then(|t| pend::accede(&t, token, need).map_err(mail::mail_fail));
    match found {
        Err(e) => Err(e),
        Ok(pie) => match usable(&pie).map_err(mail::mail_fail) {
            Err(e) => Err(e),
            Ok(()) => op(&pie),
        },
    }
}
