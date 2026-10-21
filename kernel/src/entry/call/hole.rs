use alloc::sync::Arc;
use alloc::vec::Vec;

use env::{
    HoleCall, HoleCondition, HoleFail, HoleLimits, Mark, Oversize, PieFail, PieKind, PieToken,
    PullOutcome, TaskId, Wait,
};

use super::gate;
use super::outcome::{Outcome, answer, ret_err};
use crate::core::memory::address::VirtAddr as KVirt;
use crate::core::memory::nest::Access;
use crate::core::work::mail::{self, Mail};
use crate::core::work::pend::{self, Need, Permission};
use crate::core::work::room::conductor::current;
use crate::core::work::room::messenger::Handoff;
use crate::core::work::unit::space::Space;
use crate::core::work::unit::task::TaskIdent;
use crate::machine::switcher::context::{Gprs, TrapContext};

pub(super) fn dispatch(frame: &mut TrapContext, call: HoleCall, ident: Arc<TaskIdent>) -> Outcome {
    match call {
        HoleCall::Unseal { mark, limits } => {
            answer(frame, create(mark, limits).map_err(fail));
            Outcome::Resume
        }
        HoleCall::Seal { token } => {
            answer(
                frame,
                super::seal::seal_resource(token, PieKind::Hole).map_err(fail),
            );
            Outcome::Resume
        }
        HoleCall::Push { token, msg, len } => push(frame, ident, token, msg.get(), len),
        HoleCall::Pull {
            token,
            buf,
            max,
            oversize,
        } => pull(frame, ident, token, buf.get(), max, oversize),
        HoleCall::Peek { token } => peek(frame, token),
        HoleCall::Wait {
            token,
            condition,
            millis,
        } => wait(frame, ident, token, condition, millis),
        HoleCall::Ring { token } => ring(frame, token),
        HoleCall::Hush { token } => hush(frame, token),
    }
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
    let r = with_hole(token, Need::Store, |meta| {
        if len == 0
            || !ident
                .team
                .space
                .validate_access(KVirt::wrap(msg), len, Access::READ)
        {
            return Err(HoleFail::Denied);
        }
        let reservation = mail::hole::reserve_len(meta, len)?;
        let mut cell = Vec::new();
        cell.try_reserve_exact(len).map_err(|_| HoleFail::OoM)?;
        cell.resize(len, 0);
        if !ident.team.space.copy_in(&mut cell, KVirt::wrap(msg)) {
            return Err(HoleFail::Denied);
        }
        #[cfg(all(debug_assertions, feature = "dma-confirmation-scene"))]
        if crate::entry::check::dma_confirmation::reject(
            meta.owner(),
            me,
            ident.team.life.parent.is_none(),
            &cell,
        ) {
            return Err(HoleFail::Denied);
        }
        let bytes = Arc::try_new(cell).map_err(|_| HoleFail::OoM)?;
        reservation.commit(bytes, me)
    });
    answer(frame, r.map(|()| 0));
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
    if !Space::user_range(KVirt::wrap(buf), max) {
        ret_err(frame, HoleFail::Denied);
        return Outcome::Resume;
    }
    let r = with_hole(token, Need::Fetch, |meta| {
        if !ident
            .team
            .space
            .validate_access(KVirt::wrap(buf), max, Access::WRITE)
        {
            return Err(HoleFail::Denied);
        }
        hand_over(meta, &ident.team.space, buf, max, oversize)
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
        Err(e) => ret_err(frame, e),
    }
    Outcome::Resume
}

fn hand_over(
    meta: &Arc<mail::hole::HoleMeta>,
    space: &Arc<Space>,
    buf: usize,
    max: usize,
    oversize: Oversize,
) -> Result<PullOutcome, HoleFail> {
    let reading = mail::hole::read(meta)?;
    let from = reading.from;
    let len = reading.bytes.len();
    if len > max && oversize == Oversize::Discard {
        reading.finish();
        return Ok(PullOutcome::Discarded { len, sender: from });
    }
    if len > max || !space.validate_access(KVirt::wrap(buf), len, Access::WRITE) {
        mail::hole::note_back(len, max);
        return Err(HoleFail::Denied);
    }
    if !space.copy_out(&reading.bytes, KVirt::wrap(buf)) {
        mail::hole::note_back(len, max);
        return Err(HoleFail::Denied);
    }
    reading.finish();
    Ok(PullOutcome::Received { len, sender: from })
}

fn peek(frame: &mut TrapContext, token: PieToken) -> Outcome {
    let r = with_hole(token, Need::Fetch, |meta| mail::hole::peek(meta));
    match r {
        Ok((n, from, depth)) => {
            frame.gpr.set_x(Gprs::A0, n);
            frame.gpr.set_x(Gprs::A1, from.get());
            frame.gpr.set_x(Gprs::A2, depth);
        }
        Err(e) => ret_err(frame, e),
    }
    Outcome::Resume
}

fn wait(
    frame: &mut TrapContext,
    ident: Arc<TaskIdent>,
    token: PieToken,
    condition: HoleCondition,
    millis: Wait,
) -> Outcome {
    let dur = millis.into_duration();
    let r = with_hole(token, mail::hole::need(condition), |meta| {
        frame.gpr.set_x(Gprs::A0, 0);
        drop(ident);
        mail::hole::wait(meta, condition, dur)
    });
    match r {
        Ok(Handoff::Resume(ready)) => frame.gpr.set_x(Gprs::A0, ready as usize),
        Ok(Handoff::Switch(pa)) => return Outcome::Switch(pa as *mut TrapContext),
        Err(e) => ret_err(frame, e),
    }
    Outcome::Resume
}

fn ring(frame: &mut TrapContext, token: PieToken) -> Outcome {
    let r = with_hole(token, Need::Store, |meta| mail::hole::ring(meta));
    answer(frame, r.map(|()| 0));
    Outcome::Resume
}

fn hush(frame: &mut TrapContext, token: PieToken) -> Outcome {
    let r = with_hole(token, Need::Fetch, |meta| mail::hole::hush(meta));
    answer(frame, r.map(|()| 0));
    Outcome::Resume
}

/// 取这一枚 Hole 并按本域词汇报错：失败不是 Hole 的状态就是权柄轴的状态。
fn with_hole<T>(
    token: PieToken,
    need: Need,
    operation: impl FnOnce(&Arc<mail::hole::HoleMeta>) -> Result<T, HoleFail>,
) -> Result<T, HoleFail> {
    let pie = gate::accede(token, need).map_err(fail)?;
    gate::usable(&pie).map_err(fail)?;
    let meta = pie.mail().hole().ok_or(HoleFail::Denied)?;
    operation(&meta)
}

pub(crate) fn create(mark: Mark, limits: HoleLimits) -> Result<usize, PieFail> {
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
}

fn fail(error: PieFail) -> HoleFail {
    match error {
        PieFail::Denied | PieFail::NotAligned => HoleFail::Denied,
        PieFail::Dead => HoleFail::Dead,
        PieFail::OoM => HoleFail::OoM,
        PieFail::HandedOver => HoleFail::HandedOver,
        PieFail::Busy => HoleFail::Busy,
    }
}
