use ::core::time::Duration;

use alloc::sync::Arc;

use env::{RoomCall, RoomFail};

use crate::core::work::record::trace::{self, EventKind, RoomEvent};
use crate::machine::switcher::context::{Gprs, TrapContext};
use crate::core::work::room::messenger::{self, Handoff, WakeKey, park, park_until, wait, wake};
use crate::core::work::room::scheduler::core::{current, muster};
use crate::core::work::unit::task::TaskIdent;

use super::ret_err;

pub(super) enum Outcome {
    Resume,
    Switch(*mut TrapContext),
    Exit,
}

impl Outcome {
    fn fail<E: env::FailCode>(frame: &mut TrapContext, e: E) -> Self {
        ret_err(frame, e);
        Self::Resume
    }
}

pub(super) fn dispatch(frame: &mut TrapContext, call: RoomCall, ident: Arc<TaskIdent>) -> Outcome {
    match call {
        RoomCall::Starve => Outcome::Switch(current().starve() as *mut TrapContext),
        RoomCall::Reap { reason, note, len } => {
            crate::core::work::room::messenger::set_exit_note(note.get(), len);
            crate::core::work::room::messenger::set_exit_reason(reason);
            drop(ident);
            Outcome::Exit
        }
        RoomCall::Doom { task } => {
            let Some(me) = muster(ident.id).and_then(|w| w.upgrade()) else {
                return Outcome::fail(frame, RoomFail::Denied);
            };
            if !Arc::ptr_eq(&me.ident, &ident)
                || !crate::core::work::pend::allows(
                    &me,
                    &crate::core::work::mail::Mail::Nole(super::resources::get().doom.clone()),
                    crate::core::work::pend::Need::Fetch,
                )
            {
                return Outcome::fail(frame, RoomFail::Denied);
            }
            drop(me);
            let target = muster(task).and_then(|w| w.upgrade());
            let Some(target) = target else {
                return Outcome::fail(frame, RoomFail::Dead);
            };
            let team = target.ident.team.clone();
            if team.all_reaped() {
                return Outcome::fail(frame, RoomFail::Dead);
            }
            trace::note(EventKind::Room(RoomEvent::Doomed {
                tid: target.ident.id.get(),
                by: ident.id.get(),
            }));
            drop(target);
            drop(ident);
            messenger::cull(&[team], messenger::EXIT_DOOM);
            frame.gpr.set_x(Gprs::A0, 0);
            Outcome::Resume
        }
        RoomCall::Park { millis } => {
            drop(ident);
            match park::<RoomFail>(Duration::from_millis(millis as u64)) {
                Ok(pa) => Outcome::Switch(pa as *mut TrapContext),
                Err(e) => Outcome::fail(frame, e),
            }
        }
        RoomCall::ParkUntil { at } => {
            drop(ident);
            match park_until::<RoomFail>(at) {
                Ok(None) => Outcome::Resume,
                Ok(Some(pa)) => Outcome::Switch(pa as *mut TrapContext),
                Err(e) => Outcome::fail(frame, e),
            }
        }
        RoomCall::Wait { key, millis } => {
            let (space, wlife) = {
                let s = &ident.team.space;
                (s.memory.asid(), s.life())
            };
            let wkey = WakeKey::Space { space, slot: key };
            let dur = millis.into_duration();
            drop(ident);
            match wait::<RoomFail>(wkey, wlife, dur) {
                Ok(Handoff::Resume(())) => Outcome::Resume,
                Ok(Handoff::Switch(pa)) => Outcome::Switch(pa as *mut TrapContext),
                Err(e) => Outcome::fail(frame, e),
            }
        }
        RoomCall::Wake { key } => {
            let (space, wlife) = {
                let s = &ident.team.space;
                (s.memory.asid(), s.life())
            };
            let wkey = WakeKey::Space { space, slot: key };
            let woke = wake(wkey, &wlife);
            frame.gpr.set_x(Gprs::A0, woke as usize);
            Outcome::Resume
        }
    }
}
