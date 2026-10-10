use alloc::sync::Arc;
use alloc::vec::Vec;

use env::{UnitCall, UnitFail, UnitTarget};

use crate::memory::manager::MapError;
use crate::memory::manager::addr::VirtAddr as KVirt;
use crate::runtime::switcher::context::{Gprs, TrapContext};
use crate::work::room::messenger::{self, Handoff};
use crate::work::room::scheduler::core::muster;
use crate::work::unit::life::TaskLife;
use crate::work::unit::space::{Space, SpaceKind};
use crate::work::unit::task::{MAX_ARGS, Task, TaskIdent};
use crate::work::unit::join::{JoinTarget, JoinWait};
use crate::work::unit::team::TeamState;
use crate::work::unit::weak::{Site, TaskWeak};

use super::ret_err;

pub(super) enum Outcome {
    Resume,
    Switch(*mut TrapContext),
}

impl Outcome {
    fn fail<E: env::FailCode>(frame: &mut TrapContext, e: E) -> Self {
        ret_err(frame, e);
        Self::Resume
    }
}

fn map_err(e: MapError) -> UnitFail {
    match e {
        MapError::OutOfMemory => UnitFail::OoM,
        MapError::NotAligned
        | MapError::AlreadyMapped
        | MapError::NotMapped
        | MapError::NoRegion
        | MapError::WidenDenied
        | MapError::SegmentMismatch
        | MapError::DramOverlap => UnitFail::Denied,
    }
}

fn copy_words(space: &Space, va: KVirt, count: usize) -> Option<Vec<usize>> {
    if count > MAX_ARGS {
        return None;
    }
    let width = size_of::<usize>();
    let len = count * width;
    let mut bytes = [0u8; MAX_ARGS * size_of::<usize>()];
    if !space.copy_in(&mut bytes[..len], va.as_usize()) {
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
    let Some(me) = muster(ident.id).and_then(|w| w.upgrade()) else { return Outcome::fail(frame, UnitFail::Denied) };
    match call {
        UnitCall::Spawn { team, entry, args, count, stack } => {
            let target = if team.get() == 0 { ident.team.clone() }
                else { match me.heir(team) { Some(t) => t, None => return Outcome::fail(frame, UnitFail::Denied) } };
            let Some(words) = copy_words(&ident.team.space, KVirt::wrap(args.get()), count)
                else { return Outcome::fail(frame, UnitFail::Denied) };
            match crate::work::unit::team::spawn(&target, Some(&me), entry, words, stack) {
                Ok(task) => frame.gpr.set_x(Gprs::A0, task.ident.id.get()),
                Err(error) => return Outcome::fail(frame, error),
            }
        }
        UnitCall::Build { kind } => {
            if kind == env::ProgramKind::Supervisor && !ident.team.space.kind().is_supervisor()
                || !crate::work::unit::gate::allows(&me, &super::resources::get().build, crate::work::unit::gate::Need::Fetch)
            { return Outcome::fail(frame, UnitFail::Denied); }
            let sire = TaskWeak::stored(Arc::downgrade(&me), Site::Sire);
            match crate::work::unit::build(SpaceKind::from(kind), sire) {
                Ok(team) => frame.gpr.set_x(Gprs::A0, team.id.get()),
                Err(error) => return Outcome::fail(frame, map_err(error)),
            }
        }
        UnitCall::SelfId => frame.gpr.set_x(Gprs::A0, ident.id.get()),
        UnitCall::Sire => frame.gpr.set_x(Gprs::A0, ident.team.sire().map_or(0, |id| id.get())),
        UnitCall::Scan { after, buf, capacity } => {
            if !(1..=64).contains(&capacity) || !Space::user_range(buf.get(), capacity * 8) {
                return Outcome::fail(frame, UnitFail::Denied);
            }
            let mut page = [0u64; 64]; let mut n = 0;
            {
                let _commit = crate::work::unit::commit();
                let heirs = me.heir.lock(); let mut cursor = after.get();
                while n < capacity {
                    let next = heirs.iter().map(|t| t.id.get()).filter(|id| *id > cursor).min();
                    let Some(id) = next else { break }; page[n] = id as u64; cursor = id; n += 1;
                }
            }
            let mut bytes = [0u8; 512];
            for (at, id) in page[..n].iter().enumerate() { bytes[at * 8..at * 8 + 8].copy_from_slice(&id.to_le_bytes()); }
            if !ident.team.space.copy_out(&bytes[..n * 8], buf.get()) { return Outcome::fail(frame, UnitFail::Denied); }
            frame.gpr.set_x(Gprs::A0, n);
        }
        UnitCall::Join { target, millis, receive } => {
            let _commit = crate::work::unit::commit();
            let selected = match target {
                UnitTarget::Team(id) => {
                    let Some(team) = me.heir(id) else { return Outcome::fail(frame, UnitFail::Denied) };
                    JoinTarget::Team(team.life.clone())
                }
                UnitTarget::Task(id) => {
                    if id == ident.id && millis != env::Wait::POLL { return Outcome::fail(frame, UnitFail::Denied); }
                    let mut member = if !receive { ident.team.life.member(id) } else { None };
                    let mut at = 0;
                    while member.is_none() {
                        let Some(root) = me.heir_node(at) else { break }; at += 1;
                        member = if receive { root.life.find(id).filter(|m| m.pending(ident.id)) }
                            else { root.life.member(id).or_else(|| root.life.find(id).filter(|m| m.pending(ident.id))) };
                    }
                    let Some(member) = member else { return Outcome::fail(frame, UnitFail::Denied) };
                    JoinTarget::Task(member)
                }
            };
            let join = JoinWait::new(selected, ident.id, receive, millis);
            drop(_commit);
            match messenger::unit_join(join) {
                Ok(Handoff::Resume(reply)) => JoinWait::write(frame, Ok(reply)),
                Ok(Handoff::Switch(pa)) => return Outcome::Switch(pa as *mut TrapContext),
                Err(error) => return Outcome::fail(frame, error),
            }
            // Removing the last pending owner makes metadata collectible; active
            // observers still pin their Member, not the execution resources.
            { let _commit = crate::work::unit::commit(); ident.team.life.prune();
              let mut at = 0; while let Some(root) = me.heir_node(at) { root.life.prune(); at += 1; } }
        }
        UnitCall::Oust { team } => {
            let Some(child) = me.heir(team) else { return Outcome::fail(frame, UnitFail::Denied) };
            let Some(_operation) = child.operation() else { return Outcome::fail(frame, UnitFail::Busy) };
            {
                let _commit = crate::work::unit::commit();
                if !child.all_reaped() { return Outcome::fail(frame, UnitFail::Busy); }
                // The operation lease excludes construction publication while
                // staged resources are released. Busy does not close the team.
            }
            if let Err(error) = child.cancel_staging() { return Outcome::fail(frame, map_err(error)); }
            let retired = {
                let _commit = crate::work::unit::commit();
                *child.state.lock() = TeamState::Ousted;
                child.life.clear(ident.id, Some(team)); child.life.prune(); me.oust(team)
            };
            drop(_operation); drop(retired); drop(child);
        }
        UnitCall::Fall { millis } => {
            let mine = TaskLife { id: ident.id, life: me.life() };
            match messenger::fall::<UnitFail>(mine, millis.into_duration()) {
                Ok(Handoff::Resume(landed)) => frame.gpr.set_x(Gprs::A0, landed as usize),
                Ok(Handoff::Switch(pa)) => return Outcome::Switch(pa as *mut TrapContext),
                Err(error) => return Outcome::fail(frame, error),
            }
        }
        UnitCall::Embark { target } | UnitCall::Debark { target } | UnitCall::Slay { target } => {
            let result = match target {
                UnitTarget::Task(id) => {
                    let Some(task) = muster(id).and_then(|w| w.upgrade()) else { return Outcome::fail(frame, UnitFail::Denied) };
                    if task.ident.team.id != ident.team.id && me.heir(task.ident.team.id).is_none() {
                        return Outcome::fail(frame, UnitFail::Denied);
                    }
                    match call {
                        UnitCall::Embark { .. } => Task::embark(&task),
                        UnitCall::Debark { .. } => {
                            let result = Task::debark(&task);
                            if id == ident.id && matches!(result, Err(UnitFail::Busy)) {
                                return Outcome::Switch(crate::work::room::scheduler::trap::run() as *mut TrapContext);
                            }
                            result
                        }
                        UnitCall::Slay { .. } => {
                            messenger::slay(&task);
                            if id == ident.id { return Outcome::Switch(messenger::quit() as *mut TrapContext); }
                            Ok(())
                        }
                        _ => unreachable!(),
                    }
                }
                UnitTarget::Team(id) => {
                    let Some(team) = me.heir(id) else { return Outcome::fail(frame, UnitFail::Denied) };
                    match call {
                        UnitCall::Embark { .. } => team.embark(),
                        UnitCall::Debark { .. } => team.debark(),
                        UnitCall::Slay { .. } => {
                            if !crate::work::unit::gate::allows(&me, &super::resources::get().doom, crate::work::unit::gate::Need::Fetch)
                            { return Outcome::fail(frame, UnitFail::Denied); }
                            messenger::cull(core::slice::from_ref(&team), messenger::EXIT_DOOM); Ok(())
                        }
                        _ => unreachable!(),
                    }
                }
            };
            if let Err(error) = result { return Outcome::fail(frame, error); }
        }
    }
    Outcome::Resume
}
