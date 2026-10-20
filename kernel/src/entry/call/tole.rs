use super::outcome::{Outcome, answer};
use crate::core::work::mail::{
    Mail,
    tole::{self, AwaitWait, Cell, Watch, codec},
};
use crate::core::work::pend::{self, Need};
use crate::core::work::room::conductor::current;
use crate::core::work::room::messenger::{self, Handoff, WakeKey};
use crate::core::work::room::scheduler::roster::muster;
use crate::core::work::unit::task::{Task, TaskIdent};
use crate::machine::switcher::context::TrapContext;
use alloc::sync::Arc;
use alloc::vec::Vec;
use env::{PieFail, PieKind, PieToken, ToleCall, ToleEventSource, ToleFail, Wait};

pub(super) fn dispatch(frame: &mut TrapContext, call: ToleCall, ident: Arc<TaskIdent>) -> Outcome {
    drop(ident);
    match call {
        ToleCall::Unseal { shared } => answer(frame, create(shared)),
        ToleCall::Seal { token } => answer(
            frame,
            super::seal::seal_resource(token, PieKind::Tole).map_err(codec::pie_fail),
        ),
        ToleCall::Attach { tole, source } => {
            answer(
                frame,
                attach(tole, |task| typed_cell(task, source)).map(|()| 0),
            );
        }
        ToleCall::Detach { tole, source } => {
            let _commit = crate::core::work::commit();
            let result = with_group(tole, Need::Store, |task, group| {
                tole::detach(group, task.ident.id, source)
            });
            answer(frame, result.map(|()| 0));
        }
        ToleCall::Await { tole, millis } => return await_group(frame, tole, millis),
    }
    Outcome::Resume
}

pub(crate) fn create(shared: bool) -> Result<usize, ToleFail> {
    let task = current().running_task().ok_or(ToleFail::Denied)?;
    let meta = tole::ToleMeta::try_new(task.ident.id).map_err(|_| ToleFail::OoM)?;
    let mut permission = env::Permission::FETCH | env::Permission::STORE | env::Permission::VEST;
    if !shared {
        permission |= env::Permission::ONLY;
    }
    let pie = pend::Pie::try_new(Mail::Tole(meta), env::Mark::NONE, permission, None)
        .map_err(codec::pie_fail)?;
    let token = pie.token();
    pend::insert(&task, pie).map_err(codec::pie_fail)?;
    Ok(token.get())
}

fn with_group<T>(
    token: PieToken,
    need: Need,
    operation: impl FnOnce(&Arc<Task>, &Arc<tole::ToleMeta>) -> Result<T, ToleFail>,
) -> Result<T, ToleFail> {
    let task = current().running_task().ok_or(ToleFail::Denied)?;
    let pie = pend::accede(&task, token, need).map_err(codec::pie_fail)?;
    pend::available(&pie).map_err(codec::pie_fail)?;
    let group = pie.mail().tole().ok_or(ToleFail::Denied)?;
    operation(&task, &group)
}

fn attach(
    token: PieToken,
    registration: impl FnOnce(&Arc<Task>) -> Result<Cell, ToleFail>,
) -> Result<(), ToleFail> {
    let _commit = crate::core::work::commit();
    with_group(token, Need::Store, |task, group| {
        tole::attach(group, registration(task)?)
    })
}

fn await_group(frame: &mut TrapContext, token: PieToken, millis: Wait) -> Outcome {
    let result = with_group(token, Need::Fetch, |task, group| {
        Ok(AwaitWait::new(group.clone(), token, task, millis))
    })
    .and_then(messenger::mail_await);
    match result {
        Ok(Handoff::Resume(reply)) => AwaitWait::write_result(frame, Ok(reply)),
        Ok(Handoff::Switch(pa)) => return Outcome::Switch(pa as *mut TrapContext),
        Err(error) => AwaitWait::write_result(frame, Err(error)),
    }
    Outcome::Resume
}

pub(crate) fn typed_cell(actor: &Arc<Task>, source: ToleEventSource) -> Result<Cell, ToleFail> {
    let mut keys = Vec::new();
    keys.try_reserve(4).map_err(|_| ToleFail::OoM)?;
    let watch = match source {
        ToleEventSource::Hole { .. }
        | ToleEventSource::Pole { .. }
        | ToleEventSource::Nole { .. } => {
            let (token, need) = codec::source_watch(source).ok_or(ToleFail::Denied)?;
            let pie = pend::accede(actor, token, need).map_err(codec::pie_fail)?;
            pend::available(&pie).map_err(codec::pie_fail)?;
            let (key, life) = codec::source_key(&pie, source).ok_or(ToleFail::Denied)?;
            keys.push((key, life));
            keys.push((
                WakeKey::Inspect {
                    task: actor.ident.id,
                    token: token.get(),
                },
                actor.life(),
            ));
            Watch::Resource { source }
        }
        ToleEventSource::Join { target } => {
            let selected = crate::core::work::unit::join::observe(actor, target)
                .map_err(|_| ToleFail::Denied)?;
            let wait = crate::core::work::unit::join::JoinWait::new(
                selected.clone(),
                actor.ident.id,
                false,
                Wait::POLL,
            );
            keys.push((wait.target.key(), wait.target.life()));
            Watch::Join { target, selected }
        }
        ToleEventSource::Inspect { task: id, token } => {
            let holder = muster(id)
                .and_then(|task| task.upgrade())
                .ok_or(ToleFail::Denied)?;
            let pie = pend::locate(&holder, token).ok_or(ToleFail::Denied)?;
            if !pie.alive() {
                return Err(ToleFail::Dead);
            }
            let parent = if id == actor.ident.id {
                None
            } else {
                let parent = pie.sire().ok_or(ToleFail::Denied)?;
                if !pie.lord().ptr_eq(&Arc::downgrade(actor))
                    || pend::locate(actor, parent).is_none()
                {
                    return Err(ToleFail::Denied);
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
            let held =
                parent.is_none() && matches!(pend::available(&pie), Err(PieFail::HandedOver));
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
        keys: Arc::try_new(keys).map_err(|_| ToleFail::OoM)?,
        watch,
    })
}
