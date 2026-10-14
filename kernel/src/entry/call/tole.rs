use super::outcome::{answer, ret_err, Outcome};
use crate::core::work::pend::available;
use crate::machine::switcher::context::TrapContext;
use crate::core::work::mail::{
    self,
    tole::{self, AwaitWait, Cell, Watch},
};
use crate::core::work::pend::{self, Need};
use crate::core::work::room::messenger::{self, Handoff, WakeKey};
use crate::core::work::room::conductor::current;
use crate::core::work::room::scheduler::roster::muster;
use crate::core::work::unit::task::{Task, TaskIdent};
use alloc::sync::Arc;
use alloc::vec::Vec;
use env::{MailCall, MailFail, Source};

pub(super) fn dispatch(
    frame: &mut TrapContext,
    call: MailCall,
    _: Arc<TaskIdent>,
) -> Outcome {
    let task = match current().running_task() {
        Some(task) => task,
        None => {
            ret_err(frame, MailFail::Denied);
            return Outcome::Resume;
        }
    };
    match call {
        MailCall::Attach { tole, source } => {
            let result = (|| {
                let _commit = crate::core::work::commit();
                let pie = pend::accede(&task, tole, Need::Store).map_err(mail::mail_fail)?;
                available(&pie)?;
                let group = pie.mail().tole().ok_or(MailFail::Denied)?;
                tole::attach(&group, cell(&task, source)?)
            })();
            answer(frame, result.map(|()| 0));
            Outcome::Resume
        }
        MailCall::Detach { tole, source } => {
            let result = (|| {
                let _commit = crate::core::work::commit();
                let pie = pend::accede(&task, tole, Need::Store).map_err(mail::mail_fail)?;
                available(&pie)?;
                {
                    let group = pie.mail().tole().ok_or(MailFail::Denied)?;
                    tole::detach(&group, task.ident.id, source)
                }
            })();
            answer(frame, result.map(|()| 0));
            Outcome::Resume
        }
        MailCall::Await { tole, millis } => {
            let result = (|| {
                let pie = pend::accede(&task, tole, Need::Fetch).map_err(mail::mail_fail)?;
                available(&pie)?;
                let group = pie.mail().tole().ok_or(MailFail::Denied)?;
                messenger::mail_await(AwaitWait::new(group, tole, &task, millis))
            })();
            match result {
                Ok(Handoff::Resume(reply)) => AwaitWait::write(frame, Ok(reply)),
                Ok(Handoff::Switch(pa)) => return Outcome::Switch(pa as *mut TrapContext),
                Err(e) => AwaitWait::write(frame, Err(e)),
            }
            Outcome::Resume
        }
        _ => unreachable!("mail::dispatch routes only Attach, Detach and Await"),
    }
}
pub(crate) fn cell(actor: &Arc<Task>, source: Source) -> Result<Cell, MailFail> {
    let mut keys = Vec::new();
    keys.try_reserve(4).map_err(|_| MailFail::OoM)?;
    let watch = match source {
        Source::Mail {
            pie: token,
            condition,
        } => {
            let need = Need::of(condition);
            let pie = pend::accede(actor, token, need).map_err(mail::mail_fail)?;
            available(&pie)?;
            let (key, life) = pie.mail().key(condition).ok_or(MailFail::Denied)?;
            keys.push((key, life));
            keys.push((
                WakeKey::Inspect {
                    task: actor.ident.id,
                    token: token.get(),
                },
                actor.life(),
            ));
            Watch::Mail {
                pie: token,
                condition,
            }
        }
        Source::Join { target } => {
            let selected =
                crate::core::work::unit::join::observe(actor, target).map_err(|_| MailFail::Denied)?;
            let wait = crate::core::work::unit::join::JoinWait::new(
                selected.clone(),
                actor.ident.id,
                false,
                env::Wait::POLL,
            );
            keys.push((wait.target.key(), wait.target.life()));
            Watch::Join { target, selected }
        }
        Source::Inspect { task: id, token } => {
            let holder = muster(id)
                .and_then(|t| t.upgrade())
                .ok_or(MailFail::Denied)?;
            let pie = pend::locate(&holder, token).ok_or(MailFail::Denied)?;
            if !pie.alive() {
                return Err(MailFail::Dead);
            }
            let parent = if id == actor.ident.id {
                None
            } else {
                let parent = pie.sire().ok_or(MailFail::Denied)?;
                if !pie.lord().ptr_eq(&Arc::downgrade(actor))
                    || pend::locate(actor, parent).is_none()
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
            let held = parent.is_none() && matches!(available(&pie), Err(MailFail::HandedOver));
            let mail = pie.mail();
            keys.push((WakeKey::seal(mail.kind(), mail.id()), mail.life()));
            keys.push((
                WakeKey::Inspect {
                    task: id,
                    token: token.get(),
                },
                holder.life(),
            ));
            Watch::Inspect {
                task: id,
                token,
                holder: Arc::downgrade(&holder),
                parent,
                held,
                access: pie.permission() & (env::Permission::FETCH | env::Permission::STORE),
            }
        }
    };
    Ok(Cell {
        actor: actor.ident.id,
        keys: Arc::try_new(keys).map_err(|_| MailFail::OoM)?,
        watch,
    })
}
