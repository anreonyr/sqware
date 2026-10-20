use alloc::sync::Arc;
use env::{NoleCall, NoleFail, PieFail, PieKind, PieToken, Wait};

use riscv::register::sie;

use crate::core::work::mail::{self, Mail};
use crate::core::work::pend::{self, Need, Permission};
use crate::core::work::room::conductor::current;
use crate::core::work::room::messenger::Handoff;
use crate::core::work::unit::task::TaskIdent;
use crate::machine::switcher::context::{Gprs, TrapContext};

use super::gate;
use super::outcome::{Outcome, answer, ret_err};

pub(super) fn dispatch(frame: &mut TrapContext, call: NoleCall, ident: Arc<TaskIdent>) -> Outcome {
    match call {
        NoleCall::Unseal => {
            answer(frame, create().map_err(fail));
            Outcome::Resume
        }
        NoleCall::Seal { token } => {
            answer(
                frame,
                super::seal::seal_resource(token, PieKind::Nole).map_err(fail),
            );
            Outcome::Resume
        }
        NoleCall::Wait { token, millis } => wait(frame, ident, token, millis),
        NoleCall::Ring { token } => ring(frame, token),
        NoleCall::Hush { token } => hush(frame, token),
    }
}

fn wait(frame: &mut TrapContext, ident: Arc<TaskIdent>, token: PieToken, millis: Wait) -> Outcome {
    let dur = millis.into_duration();
    let r = with_nole(token, Need::Fetch, |meta| {
        frame.gpr.set_x(Gprs::A0, 0);
        drop(ident);
        mail::nole::wait(meta, dur)
    });
    match r {
        Ok(Handoff::Resume(ready)) => frame.gpr.set_x(Gprs::A0, ready as usize),
        Ok(Handoff::Switch(pa)) => return Outcome::Switch(pa as *mut TrapContext),
        Err(e) => ret_err(frame, e),
    }
    Outcome::Resume
}

fn ring(frame: &mut TrapContext, token: PieToken) -> Outcome {
    let r = with_nole(token, Need::Store, |meta| mail::nole::ring(meta));
    answer(frame, r.map(|()| 0));
    Outcome::Resume
}

/// 放声同时把外部中断这条线打开：Nole 的敲门位就在它上面。
fn hush(frame: &mut TrapContext, token: PieToken) -> Outcome {
    let r = with_nole(token, Need::Fetch, |meta| mail::nole::hush(meta));
    if r.is_ok() {
        unsafe {
            sie::set_sext();
        }
    }
    answer(frame, r.map(|()| 0));
    Outcome::Resume
}

fn with_nole<T>(
    token: PieToken,
    need: Need,
    operation: impl FnOnce(&Arc<mail::nole::NoleMeta>) -> Result<T, NoleFail>,
) -> Result<T, NoleFail> {
    let pie = gate::accede(token, need).map_err(fail)?;
    gate::usable(&pie).map_err(fail)?;
    let meta = pie.mail().nole().ok_or(NoleFail::Denied)?;
    operation(&meta)
}

pub(crate) fn create() -> Result<usize, PieFail> {
    let task = current().running_task().ok_or(PieFail::Denied)?;
    let meta = mail::nole::NoleMeta::try_new(task.ident.id).map_err(|_| PieFail::OoM)?;
    let pie = pend::Pie::try_new(
        Mail::Nole(meta),
        env::Mark::NONE,
        Permission::FETCH | Permission::STORE | Permission::VEST,
        None,
    )?;
    let token = pie.token();
    pend::insert(&task, pie)?;
    Ok(token.get())
}

fn fail(error: PieFail) -> NoleFail {
    match error {
        PieFail::Denied | PieFail::NotAligned => NoleFail::Denied,
        PieFail::Dead => NoleFail::Dead,
        PieFail::OoM => NoleFail::OoM,
        PieFail::HandedOver => NoleFail::HandedOver,
        PieFail::Busy => NoleFail::Busy,
    }
}
