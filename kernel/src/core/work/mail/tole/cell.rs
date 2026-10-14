use alloc::sync::{Arc, Weak};
use alloc::vec::Vec;

use env::{MailFail, Source, TaskId};

use crate::core::work::pend;
use crate::core::work::room::messenger::WakeKey;
use crate::core::work::unit::join::{JoinTarget, JoinWait};
use crate::core::work::unit::life::Life;
use crate::core::work::unit::task::Task;

use super::watch::Watch;

#[derive(Clone)]
pub(crate) struct Cell {
    pub actor: TaskId,
    pub keys: Arc<Vec<(WakeKey, Weak<Life>)>>,
    pub watch: Watch,
}

impl Cell {
    pub(super) fn watches(&self, actor: TaskId, source: Source) -> bool {
        self.actor == actor && self.watch.source() == source
    }

    pub(super) fn covers(&self, key: WakeKey) -> bool {
        self.keys.iter().any(|(known, _)| *known == key)
    }

    pub(super) fn poll(&self, actor: &Arc<Task>) -> Option<Option<MailFail>> {
        match &self.watch {
            Watch::Mail { pie, condition } => {
                let need = pend::Need::of(*condition);
                let snapshot = match pend::accede(actor, *pie, need) {
                    Ok(p) => p,
                    Err(e) => return Some(Some(crate::core::work::mail::mail_fail(e))),
                };
                if let Err(e) = pend::available(&snapshot) {
                    return Some(Some(e));
                }
                let ready = snapshot.mail().ready(*condition);
                ready.then_some(None)
            }
            Watch::Join { selected, .. } => {
                if let JoinTarget::Team(team) = selected {
                    if team.next(actor.ident.id).is_some() {
                        return Some(None);
                    }
                }
                let join = JoinWait::new(selected.clone(), actor.ident.id, false, env::Wait::POLL);
                match join.poll() {
                    Ok(env::JoinReply::Pending) => None,
                    Ok(_) => Some(None),
                    Err(_) => Some(Some(MailFail::Denied)),
                }
            }
            Watch::Inspect {
                token,
                holder,
                parent,
                held,
                access,
                ..
            } => {
                let Some(holder) = holder.upgrade() else {
                    return Some(Some(MailFail::Denied));
                };
                let Some(pie) = pend::locate(&holder, *token) else {
                    return Some(Some(MailFail::Denied));
                };
                if !pie.alive() {
                    return Some(Some(MailFail::Dead));
                }
                if !pie.permission().contains(*access) {
                    return Some(Some(MailFail::Denied));
                }
                if let Some((lord, parent_token)) = parent {
                    let Some(lord) = lord.upgrade() else {
                        return Some(Some(MailFail::Denied));
                    };
                    if lord.ident.id != actor.ident.id
                        || pend::locate(&lord, *parent_token).is_none()
                        || pie.sire() != Some(*parent_token)
                        || !pie.lord().ptr_eq(&Arc::downgrade(&lord))
                    {
                        return Some(Some(MailFail::Denied));
                    }
                    match pend::available(&pie) {
                        Ok(()) => None,
                        Err(e) => Some(Some(e)),
                    }
                } else {
                    match pend::available(&pie) {
                        Ok(()) => (*held).then_some(None),
                        Err(MailFail::HandedOver) if *held => None,
                        Err(e) => Some(Some(e)),
                    }
                }
            }
        }
    }
}
