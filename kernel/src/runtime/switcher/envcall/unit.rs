use alloc::sync::{Arc, Weak};
use alloc::vec::Vec;

use env::{TaskId, UnitCall, UnitFail};

use crate::memory::manager::MapError;
use crate::memory::manager::addr::VirtAddr as KVirt;
use crate::runtime::switcher::context::{Gprs, TrapContext};
use crate::work::room::messenger::{self, Handoff};
use crate::work::room::scheduler::core::{current, muster};
use crate::work::unit::life::TaskLife;
use crate::work::unit::space::{Space, SpaceKind};
use crate::work::unit::task::{MAX_ARGS, Task, TaskIdent, TaskTag};
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
                match current().running_task().and_then(|me| me.heir(team)) {
                    Some(t) => t,
                    None => return Outcome::fail(frame, UnitFail::Denied),
                }
            };
            let words = match copy_words(&ident.team.space, KVirt::wrap(args.get()), count) {
                Some(w) => w,
                None => return Outcome::fail(frame, UnitFail::Denied),
            };
            let caller = current().running_task();
            let result = crate::work::unit::team::spawn(&target, caller.as_ref(), entry, words, stack);
            match result {
                Ok(task) => frame.gpr.set_x(Gprs::A0, task.ident.id.get()),
                Err(error) => return Outcome::fail(frame, error),
            }
            Outcome::Resume
        }
        UnitCall::SelfId => {
            let id = current()
                .running_task()
                .map(|t| t.ident.id)
                .unwrap_or(TaskId::new(0));
            frame.gpr.set_x(Gprs::A0, id.get());
            Outcome::Resume
        }
        UnitCall::Sire => {
            let id = current()
                .running_task()
                .and_then(|t| t.ident.team.sire())
                .unwrap_or(TaskId::new(0));
            frame.gpr.set_x(Gprs::A0, id.get());
            Outcome::Resume
        }
        UnitCall::HeirCount => {
            let n = current()
                .running_task()
                .map(|t| t.heir_count())
                .unwrap_or(0);
            frame.gpr.set_x(Gprs::A0, n);
            Outcome::Resume
        }
        UnitCall::Heir { index } => {
            let id = current()
                .running_task()
                .and_then(|t| t.heir_at(index))
                .map(|t| t.get())
                .unwrap_or(0);
            frame.gpr.set_x(Gprs::A0, id);
            Outcome::Resume
        }
        UnitCall::Build { kind } => {
            if kind == env::ProgramKind::Supervisor && !ident.team.space.kind().is_supervisor() {
                return Outcome::fail(frame, UnitFail::Denied);
            }
            let sire = match current().running_task() {
                Some(me) => TaskWeak::stored(Arc::downgrade(&me), Site::Sire),
                None => TaskWeak::empty(),
            };
            match crate::work::unit::build(SpaceKind::from(kind), sire) {
                Ok(team) => frame.gpr.set_x(Gprs::A0, team.id.get()),
                Err(error) => return Outcome::fail(frame, map_err(error)),
            }
            Outcome::Resume
        }
        UnitCall::Hatch { task } => {
            let target = match muster(task).and_then(|w| w.upgrade()) {
                Some(t) => t,
                None => return Outcome::fail(frame, UnitFail::Denied),
            };
            let same = Arc::ptr_eq(&target.ident.team, &ident.team);
            let mine = current()
                .running_task()
                .map(|me| me.heir(target.ident.team.id).is_some())
                .unwrap_or(false);
            if !(same || mine) {
                return Outcome::fail(frame, UnitFail::Denied);
            }
            if let Err(e) = Task::release(&target) {
                return Outcome::fail(frame, e);
            }
            Outcome::Resume
        }
        UnitCall::Join { task, millis } => {
            let dur = millis.into_duration();
            let Some(target) = muster(task) else {
                return Outcome::fail(frame, UnitFail::Denied);
            };
            let (reaped, life) = match target.upgrade() {
                Some(t) => {
                    let same = Arc::ptr_eq(&t.ident.team, &ident.team);
                    let mine = current()
                        .running_task()
                        .map(|me| me.heir(t.ident.team.id).is_some())
                        .unwrap_or(false);
                    if !(same || mine) {
                        return Outcome::fail(frame, UnitFail::Denied);
                    }
                    (t.tag() == TaskTag::Reaped, t.life())
                }
                None => (true, Weak::new()),
            };
            frame.gpr.set_x(Gprs::A0, 0);
            drop(ident);
            drop(target);
            match messenger::join::<UnitFail>(TaskLife { id: task, life }, reaped, dur) {
                Ok(Handoff::Resume(dead)) => {
                    frame.gpr.set_x(Gprs::A0, dead as usize);
                    Outcome::Resume
                }
                Ok(Handoff::Switch(pa)) => Outcome::Switch(pa as *mut TrapContext),
                Err(e) => Outcome::fail(frame, e),
            }
        }
        UnitCall::Fall { millis } => {
            let dur = millis.into_duration();
            let Some(me) = current().running_task() else {
                return Outcome::fail(frame, UnitFail::Busy);
            };
            let mine = TaskLife {
                id: me.ident.id,
                life: me.life(),
            };
            frame.gpr.set_x(Gprs::A0, 0);
            drop(ident);
            drop(me);
            match messenger::fall::<UnitFail>(mine, dur) {
                Ok(Handoff::Resume(landed)) => {
                    frame.gpr.set_x(Gprs::A0, landed as usize);
                    Outcome::Resume
                }
                Ok(Handoff::Switch(pa)) => Outcome::Switch(pa as *mut TrapContext),
                Err(e) => Outcome::fail(frame, e),
            }
        }
        UnitCall::Oust { team } => {
            let Some(me) = current().running_task() else {
                return Outcome::fail(frame, UnitFail::Denied);
            };
            let Some(child) = me.heir(team) else {
                return Outcome::fail(frame, UnitFail::Denied);
            };
            let Some(_construction) = child.operation() else {
                return Outcome::fail(frame, UnitFail::Busy);
            };
            if !child.all_reaped() {
                return Outcome::fail(frame, UnitFail::Busy);
            }
            if let Err(error) = child.cancel_staging() {
                return Outcome::fail(frame, map_err(error));
            }
            drop(child);
            drop(me.oust(team));
            Outcome::Resume
        }
    }
}
