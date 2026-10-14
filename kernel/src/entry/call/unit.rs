use alloc::sync::Arc;
use alloc::vec::Vec;

use env::{UnitCall, UnitFail, UnitTarget};

use crate::core::memory::manager::MapError;
use crate::core::memory::manager::addr::VirtAddr as KVirt;
use crate::machine::switcher::context::{Gprs, TrapContext};
use crate::core::work::room::messenger::{self, Handoff};
use crate::core::work::room::scheduler::roster::muster;
use crate::core::work::unit::copies::Origin;
use crate::core::work::unit::join::JoinWait;
use crate::core::work::unit::life::TaskLife;
use crate::core::memory::manager::{Memory, SpaceKind};
use crate::core::work::unit::space::Space;
use crate::core::work::unit::task::{MAX_ARGS, Task, TaskIdent, TaskWeak};

use super::outcome::Outcome;

fn map_err(e: MapError) -> UnitFail {
    match e {
        MapError::OutOfMemory => UnitFail::OoM,
        MapError::NotAligned
        | MapError::AlreadyMapped
        | MapError::NotMapped
        | MapError::NoRegion
        | MapError::WidenDenied
        | MapError::DramOverlap
        | MapError::Shootdown
        | MapError::Pinned => UnitFail::Denied,
    }
}

fn copy_words(space: &Space, va: KVirt, count: usize) -> Option<Vec<usize>> {
    if count > MAX_ARGS {
        return None;
    }
    let width = size_of::<usize>();
    let len = count * width;
    let mut bytes = [0u8; MAX_ARGS * size_of::<usize>()];
    if !space.memory.copy_in(&mut bytes[..len], va.as_usize()) {
        return None;
    }
    let mut out: Vec<usize> = Vec::new();
    out.try_reserve(count).ok()?;
    for i in 0..count {
        let mut w = [0u8; size_of::<usize>()];
        w.copy_from_slice(&bytes[i * width..(i + 1) * width]);
        out.push(usize::from_le_bytes(w));
    }
    Some(out)
}

pub(super) fn dispatch(frame: &mut TrapContext, call: UnitCall, ident: Arc<TaskIdent>) -> Outcome {
    frame.gpr.set_x(Gprs::A0, 0);
    let Some(me) = muster(ident.id).and_then(|w| w.upgrade()) else {
        return Outcome::fail(frame, UnitFail::Denied);
    };
    match call {
        UnitCall::Spawn {
            team,
            entry,
            args,
            count,
            stack,
        } => {
            let target = if team.get() == 0 {
                ident.team.clone()
            } else {
                match me.clan(team) {
                    Some(t) => t,
                    None => return Outcome::fail(frame, UnitFail::Denied),
                }
            };
            let Some(words) = copy_words(&ident.team.space, KVirt::wrap(args.get()), count) else {
                return Outcome::fail(frame, UnitFail::Denied);
            };
            match crate::core::work::unit::team::spawn(&target, Some(&me), entry, words, stack) {
                Ok(task) => frame.gpr.set_x(Gprs::A0, task.ident.id.get()),
                Err(error) => return Outcome::fail(frame, error),
            }
        }
        UnitCall::Build { kind, sire } => {
            if kind == env::ProgramKind::Supervisor && !ident.team.space.kind().is_supervisor()
                || !crate::core::work::pend::allows(
                    &me,
                    &crate::core::work::mail::Mail::Nole(super::resources::get().build.clone()),
                    crate::core::work::pend::Need::Fetch,
                )
            {
                return Outcome::fail(frame, UnitFail::Denied);
            }
            let Some(parent) = muster(sire).and_then(|w| w.upgrade()) else {
                return Outcome::fail(frame, UnitFail::Denied);
            };
            if parent.dead()
                || parent.ident.team.id != ident.team.id && me.clan(parent.ident.team.id).is_none()
            {
                return Outcome::fail(frame, UnitFail::Denied);
            }
            let sire = TaskWeak::stored(Arc::downgrade(&parent), Origin::Sire);
            match crate::core::work::unit::team::TeamBuilder::build(SpaceKind::from(kind), sire) {
                Ok(team) => frame.gpr.set_x(Gprs::A0, team.id.get()),
                Err(error) => return Outcome::fail(frame, map_err(error)),
            }
        }
        UnitCall::SelfId => frame.gpr.set_x(Gprs::A0, ident.id.get()),
        UnitCall::Sire => frame
            .gpr
            .set_x(Gprs::A0, ident.team.sire().map_or(0, |id| id.get())),
        UnitCall::Scan {
            after,
            buf,
            capacity,
        } => {
            if !(1..=64).contains(&capacity) || !Memory::user_range(buf.get(), capacity * 8) {
                return Outcome::fail(frame, UnitFail::Denied);
            }
            let mut page = [0u64; 64];
            let mut n = 0;
            {
                let _commit = crate::core::work::commit();
                let heirs = me.heir.lock();
                let mut cursor = after.get();
                while n < capacity {
                    let next = heirs
                        .iter()
                        .map(|t| t.id.get())
                        .filter(|id| *id > cursor)
                        .min();
                    let Some(id) = next else { break };
                    page[n] = id as u64;
                    cursor = id;
                    n += 1;
                }
            }
            let mut bytes = [0u8; 512];
            for (at, id) in page[..n].iter().enumerate() {
                bytes[at * 8..at * 8 + 8].copy_from_slice(&id.to_le_bytes());
            }
            if !ident.team.space.memory.copy_out(&bytes[..n * 8], buf.get()) {
                return Outcome::fail(frame, UnitFail::Denied);
            }
            frame.gpr.set_x(Gprs::A0, n);
        }
        UnitCall::Join {
            target,
            millis,
            receive,
        } => {
            if target == UnitTarget::Task(ident.id) && millis != env::Wait::POLL {
                return Outcome::fail(frame, UnitFail::Denied);
            }
            let selected = match crate::core::work::unit::join::observe(&me, target) {
                Ok(target) => target,
                Err(error) => return Outcome::fail(frame, error),
            };
            let join = JoinWait::new(selected, ident.id, receive, millis);
            match messenger::unit_join(join) {
                Ok(Handoff::Resume(reply)) => JoinWait::write(frame, Ok(reply)),
                Ok(Handoff::Switch(pa)) => return Outcome::Switch(pa as *mut TrapContext),
                Err(error) => return Outcome::fail(frame, error),
            }
            {
                let _commit = crate::core::work::commit();
                ident.team.life.prune();
                let mut at = 0;
                while let Some(root) = me.heir_node(at) {
                    root.life.prune();
                    at += 1;
                }
            }
        }
        UnitCall::Oust { team } => {
            if let Err(error) = crate::core::work::unit::team::oust(&me, team) {
                return Outcome::fail(frame, error);
            }
        }
        UnitCall::Fall { millis } => {
            let mine = TaskLife {
                id: ident.id,
                life: me.life(),
            };
            match messenger::fall::<UnitFail>(mine, millis.into_duration()) {
                Ok(Handoff::Resume(landed)) => frame.gpr.set_x(Gprs::A0, landed as usize),
                Ok(Handoff::Switch(pa)) => return Outcome::Switch(pa as *mut TrapContext),
                Err(error) => return Outcome::fail(frame, error),
            }
        }
        UnitCall::Embark { target } | UnitCall::Debark { target } | UnitCall::Slay { target } => {
            let result = match target {
                UnitTarget::Task(id) => {
                    let Some(task) = muster(id).and_then(|w| w.upgrade()) else {
                        return Outcome::fail(frame, UnitFail::Denied);
                    };
                    if task.ident.team.id != ident.team.id && me.clan(task.ident.team.id).is_none()
                    {
                        return Outcome::fail(frame, UnitFail::Denied);
                    }
                    match call {
                        UnitCall::Embark { .. } => Task::embark(&task),
                        UnitCall::Debark { .. } => {
                            let result = Task::debark(&task);
                            if id == ident.id && matches!(result, Err(UnitFail::Busy)) {
                                return Outcome::Switch(
                                    crate::core::work::room::conductor::run::run() as *mut TrapContext
                                );
                            }
                            result
                        }
                        UnitCall::Slay { .. } => {
                            messenger::slay(&task);
                            if id == ident.id {
                                return Outcome::Switch(messenger::quit() as *mut TrapContext);
                            }
                            Ok(())
                        }
                        _ => unreachable!(),
                    }
                }
                UnitTarget::Team(id) => {
                    let Some(team) = me.clan(id) else {
                        return Outcome::fail(frame, UnitFail::Denied);
                    };
                    match call {
                        UnitCall::Embark { .. } => team.embark(),
                        UnitCall::Debark { .. } => team.debark(),
                        UnitCall::Slay { .. } => {
                            messenger::cull(::core::slice::from_ref(&team), messenger::EXIT_DOOM);
                            Ok(())
                        }
                        _ => unreachable!(),
                    }
                }
            };
            if let Err(error) = result {
                return Outcome::fail(frame, error);
            }
        }
    }
    Outcome::Resume
}
