use super::mail::Outcome;
use super::pie::observe;
use crate::runtime::switcher::context::{Gprs, TrapContext};
use crate::work::mail::tole::{self, AwaitWait, Cell, Member};
use crate::work::room::messenger::{self, Handoff, WakeKey};
use crate::work::room::scheduler::core::{current, muster};
use crate::work::unit::{
    gate::{self, Need},
    task::{Task, TaskIdent},
};
use alloc::sync::Arc;
use alloc::vec::Vec;
use env::{MailCall, MailCondition, MailFail, Source};

pub(crate) fn dispatch(
    frame: &mut TrapContext,
    call: MailCall,
    _: Arc<TaskIdent>,
) -> Option<Outcome> {
    let task = match current().running_task() {
        Some(task) => task,
        None => {
            answer(frame, Err(MailFail::Denied));
            return Some(Outcome::Resume);
        }
    };
    match call {
        MailCall::Attach { tole, source } => {
            let result = (|| {
                let _commit = crate::work::unit::commit();
                let pie = gate::accede::<MailFail>(&task, tole, Need::Store)?;
                observe(&pie)?;
                let group = pie.tole().ok_or(MailFail::Denied)?;
                tole::attach(&group, cell(&task, source)?)
            })();
            answer(frame, result);
            Some(Outcome::Resume)
        }
        MailCall::Detach { tole, source } => {
            let result = (|| {
                let _commit = crate::work::unit::commit();
                let pie = gate::accede::<MailFail>(&task, tole, Need::Store)?;
                observe(&pie)?;
                {
                    let group = pie.tole().ok_or(MailFail::Denied)?;
                    tole::detach(&group, task.ident.id, source)
                }
            })();
            answer(frame, result);
            Some(Outcome::Resume)
        }
        MailCall::Await { tole, millis } => {
            let result = (|| {
                let pie = gate::accede::<MailFail>(&task, tole, Need::Fetch)?;
                observe(&pie)?;
                let group = pie.tole().ok_or(MailFail::Denied)?;
                messenger::mail_await(AwaitWait::new(group, tole, &task, millis))
            })();
            match result {
                Ok(Handoff::Resume(reply)) => AwaitWait::write(frame, Ok(reply)),
                Ok(Handoff::Switch(pa)) => return Some(Outcome::Park(pa as *mut TrapContext)),
                Err(e) => AwaitWait::write(frame, Err(e)),
            }
            Some(Outcome::Resume)
        }
        _ => None,
    }
}
pub(crate) fn cell(actor: &Arc<Task>, source: Source) -> Result<Cell, MailFail> {
    let mut keys = Vec::new();
    keys.try_reserve(4).map_err(|_| MailFail::OoM)?;
    let member = match source {
        Source::Mail {
            pie: token,
            condition,
        } => {
            let need = if matches!(condition, MailCondition::Pull | MailCondition::Signal(_)) {
                Need::Fetch
            } else {
                Need::Store
            };
            let pie = gate::accede::<MailFail>(actor, token, need)?;
            observe(&pie)?;
            let (key, life) = if let Some(meta) = pie.hole() {
                if matches!(condition, MailCondition::Signal(_)) {
                    return Err(MailFail::Denied);
                }
                (crate::work::mail::hole::key(&meta, condition), meta.life())
            } else if let Some(meta) = pie.nole() {
                if condition != MailCondition::Pull {
                    return Err(MailFail::Denied);
                }
                (crate::work::mail::nole::key(&meta), meta.life())
            } else if let (Some(meta), MailCondition::Signal(bit)) = (pie.pole(), condition) {
                (crate::work::mail::pole::key(&meta, bit), meta.life())
            } else {
                return Err(MailFail::Denied);
            };
            keys.push((key, life));
            keys.push((
                WakeKey::Inspect {
                    task: actor.ident.id,
                    token: token.get(),
                },
                actor.life(),
            ));
            Member::Mail
        }
        Source::Join { target } => {
            let selected =
                crate::work::unit::join::observe(actor, target).map_err(|_| MailFail::Denied)?;
            let wait = crate::work::unit::join::JoinWait::new(
                selected.clone(),
                actor.ident.id,
                false,
                env::Wait::POLL,
            );
            keys.push((wait.key(), wait.life()));
            Member::Join(selected)
        }
        Source::Inspect { task: id, token } => {
            let holder = muster(id)
                .and_then(|t| t.upgrade())
                .ok_or(MailFail::Denied)?;
            let pie = gate::locate(&holder, token).ok_or(MailFail::Denied)?;
            if !pie.alive() {
                return Err(MailFail::Dead);
            }
            let parent = if id == actor.ident.id {
                None
            } else {
                let parent = pie.sire().ok_or(MailFail::Denied)?;
                if !pie.lord().ptr_eq(&Arc::downgrade(actor))
                    || gate::locate(actor, parent).is_none()
                {
                    return Err(MailFail::Denied);
                }
                keys.push((
                    WakeKey::Inspect {
                        task: actor.ident.id,
                        token: parent.get(),
                    },
                    actor.life(),
                ));
                Some((Arc::downgrade(actor), parent))
            };
            let held = parent.is_none() && matches!(observe(&pie), Err(MailFail::HandedOver));
            let resource = if let Some(meta) = pie.hole() {
                (
                    WakeKey::Seal {
                        kind: env::PieKind::Hole as u8,
                        id: meta.id().0,
                    },
                    meta.life(),
                )
            } else if let Some(meta) = pie.pole() {
                (
                    WakeKey::Seal {
                        kind: env::PieKind::Pole as u8,
                        id: meta.id().0,
                    },
                    meta.life(),
                )
            } else if let Some(meta) = pie.nole() {
                (
                    WakeKey::Seal {
                        kind: env::PieKind::Nole as u8,
                        id: meta.id().0,
                    },
                    meta.life(),
                )
            } else if let Some(meta) = pie.tole() {
                (
                    WakeKey::Seal {
                        kind: env::PieKind::Tole as u8,
                        id: meta.id().0,
                    },
                    meta.life(),
                )
            } else {
                return Err(MailFail::Denied);
            };
            keys.push(resource);
            keys.push((
                WakeKey::Inspect {
                    task: id,
                    token: token.get(),
                },
                holder.life(),
            ));
            Member::Inspect {
                holder: Arc::downgrade(&holder),
                parent,
                held,
                access: pie.permission() & (env::Permission::FETCH | env::Permission::STORE),
            }
        }
    };
    Ok(Cell {
        source,
        actor: actor.ident.id,
        member,
        keys: Arc::try_new(keys).map_err(|_| MailFail::OoM)?,
    })
}
fn answer(frame: &mut TrapContext, result: Result<(), MailFail>) {
    frame.gpr.set_x(
        Gprs::A0,
        result.map_or_else(|e| env::FailCode::code(e) as usize, |_| 0),
    );
}
