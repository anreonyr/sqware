use alloc::sync::Arc;
use alloc::vec::Vec;

use env::{HoleDir, MailCall, MailFail, PieToken, TaskId, Wait};

use riscv::register::sie;

use crate::memory::PAGE_SIZE;
use crate::memory::manager::addr::VirtAddr as KVirt;
use crate::runtime::switcher::context::{Gprs, TrapContext};
use crate::work::mail;
use crate::work::room::messenger::Handoff;
use crate::work::room::scheduler::core::current;
use crate::work::unit::gate::{self, AnyPie, Need};

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
        MailCall::Push { token, msg, len } => {
            push(frame, ident, token, KVirt::from_raw(msg.get()), len)
        }
        MailCall::Pull { token, buf, max } => {
            pull(frame, ident, token, KVirt::from_raw(buf.get()), max)
        }
        MailCall::Wait { token, dir, millis } => wait_dir(frame, ident, token, dir, millis),
        MailCall::Hush { token } => hush(frame, token),
        MailCall::Ring { token } => ring(frame, token),
    })
}

fn push(
    frame: &mut TrapContext,
    ident: Arc<TaskIdent>,
    token: PieToken,
    msg: KVirt,
    len: usize,
) -> Outcome {
    let me = current()
        .running_task()
        .map(|t| t.ident.id)
        .unwrap_or(TaskId::new(0));
    let found = current()
        .running_task()
        .ok_or(MailFail::Denied)
        .and_then(|t| gate::accede::<MailFail>(&t, token, Need::Store));
    let r = match found {
        Err(e) => Err(e),
        Ok(pie) => match usable::<MailFail>(&pie) {
            Err(e) => Err(e),
            Ok(()) => match &pie {
                AnyPie::Hole(p) => {
                    let meta = p.meta().clone();
                    if !(1..=PAGE_SIZE).contains(&len) {
                        Err(MailFail::Denied)
                    } else {
                        let mut staging: Vec<u8> = Vec::new();
                        if staging.try_reserve(len).is_err() {
                            Err(MailFail::OoM)
                        } else {
                            staging.resize(len, 0);
                            if mail::copy_in(&ident.team.space, &mut staging, msg.as_usize()) {
                                mail::hole::try_push(&meta, &mut staging, me)
                            } else {
                                Err(MailFail::Denied)
                            }
                        }
                    }
                }
                _ => Err(MailFail::Denied),
            },
        },
    };
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
    buf: KVirt,
    max: usize,
) -> Outcome {
    let found = current()
        .running_task()
        .ok_or(MailFail::Denied)
        .and_then(|t| gate::accede::<MailFail>(&t, token, Need::Fetch));
    let r = match found {
        Err(e) => Err(e),
        Ok(pie) => match usable::<MailFail>(&pie) {
            Err(e) => Err(e),
            Ok(()) => match &pie {
                AnyPie::Hole(p) => {
                    let meta = p.meta().clone();
                    if max == 0 {
                        mail::hole::peek(&meta)
                    } else {
                        match mail::hole::try_pull(&meta, max) {
                            Ok((msg, from)) => {
                                if mail::copy_out(&ident.team.space, &msg, buf.as_usize()) {
                                    Ok((msg.len(), from))
                                } else {
                                    Err(MailFail::Denied)
                                }
                            }
                            Err(e) => Err(e),
                        }
                    }
                }
                _ => Err(MailFail::Denied),
            },
        },
    };
    match r {
        Ok((n, from)) => {
            frame.gpr.set_x(Gprs::A0, n);
            frame.gpr.set_x(Gprs::A1, from.get());
        }
        Err(e) => frame.gpr.set_x(Gprs::A0, e.code() as usize),
    }
    Outcome::Resume
}

fn wait_dir(
    frame: &mut TrapContext,
    ident: Arc<TaskIdent>,
    token: PieToken,
    dir: HoleDir,
    millis: Wait,
) -> Outcome {
    enum Ready {
        Hole(Arc<mail::hole::HoleMeta>),
        Bell(Arc<mail::nole::NoleMeta>),
    }
    let need = match dir {
        HoleDir::Pull => Need::Fetch,
        HoleDir::Push => Need::Store,
    };
    let found = current()
        .running_task()
        .ok_or(MailFail::Denied)
        .and_then(|t| gate::accede::<MailFail>(&t, token, need));
    let resolved = match found {
        Err(e) => Err(e),
        Ok(pie) => match usable::<MailFail>(&pie) {
            Err(e) => Err(e),
            Ok(()) => match &pie {
                AnyPie::Hole(p) => Ok(Ready::Hole(p.meta().clone())),
                AnyPie::Nole(p) if dir == HoleDir::Pull => Ok(Ready::Bell(p.meta().clone())),
                _ => Err(MailFail::Denied),
            },
        },
    };
    let dur = millis.into_duration();
    match resolved {
        Err(e) => frame.gpr.set_x(Gprs::A0, e.code() as usize),
        Ok(ready) => {
            frame.gpr.set_x(Gprs::A0, 0);
            drop(ident);
            let parked = match &ready {
                Ready::Hole(meta) => mail::hole::wait(meta, dir, dur),
                Ready::Bell(meta) => mail::nole::wait(meta, dur),
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
    let r = with_bell(token, Need::Fetch, mail::nole::hush);
    if r.is_ok() {
        // SAFETY: 仅置本 hart SEIE 位
        unsafe {
            sie::set_sext();
        }
    }
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
    let r = with_bell(token, Need::Store, mail::nole::ring);
    frame.gpr.set_x(
        Gprs::A0,
        match r {
            Ok(()) => 0,
            Err(e) => e.code() as usize,
        },
    );
    Outcome::Resume
}

fn with_bell(
    token: PieToken,
    need: Need,
    op: fn(&mail::nole::NoleMeta) -> Result<(), MailFail>,
) -> Result<(), MailFail> {
    let found = current()
        .running_task()
        .ok_or(MailFail::Denied)
        .and_then(|t| gate::accede::<MailFail>(&t, token, need));
    match found {
        Err(e) => Err(e),
        Ok(pie) => match usable::<MailFail>(&pie) {
            Err(e) => Err(e),
            Ok(()) => match &pie {
                AnyPie::Nole(p) => op(p.meta()),
                _ => Err(MailFail::Denied),
            },
        },
    }
}
