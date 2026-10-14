use core::ops::Deref;
use core::sync::atomic::{AtomicUsize, Ordering};

use alloc::sync::{Arc, Weak};

use env::{Mark, PieToken, TaskId};

use crate::core::work::grant::narrows;
use crate::core::work::mail::Mail;
use crate::core::work::unit::task::Task;

use super::Need;

mod heir;
mod snapshot;
pub(crate) use heir::Heir;
pub(crate) use snapshot::PieSnapshot;

pub(crate) struct Pie {
    core: PieSnapshot,
}

impl Pie {
    pub(crate) fn try_new(
        meta: Mail,
        mark: Mark,
        permission: Permission,
        sire: Option<PieToken>,
    ) -> Result<Pie, env::PieFail> {
        let token = next_id();
        Ok(Pie {
            core: PieSnapshot {
                token,
                grant: meta.new_grant(permission, token)?,
                mark,
                sire,
                lord: Weak::new(),
                heir: None,
                meta,
            },
        })
    }

    pub(crate) fn snapshot(&self) -> PieSnapshot {
        self.core.clone()
    }

    pub(crate) fn invalidate(&self) {
        self.core.grant.invalidate();
    }

    pub(crate) fn parent(&mut self, token: PieToken, task: Weak<Task>) {
        self.core.sire = Some(token);
        self.core.lord = task;
    }

    pub(crate) fn set_heir(&mut self, heir: Option<Heir>) {
        self.core.heir = heir;
    }

    pub(crate) fn narrow(&mut self, subset: Permission) -> Result<(), env::PieFail> {
        if !self.core.alive() {
            return Err(env::PieFail::Dead);
        }
        if !narrows(self.core.permission(), subset) {
            return Err(env::PieFail::Denied);
        }
        self.core.grant.narrow(subset)
    }
}

impl Deref for Pie {
    type Target = PieSnapshot;
    fn deref(&self) -> &PieSnapshot {
        &self.core
    }
}

impl Drop for Pie {
    fn drop(&mut self) {
        self.core.grant.invalidate();
        self.core.meta.release_grant(self.core.token);
    }
}

unsafe impl Send for Pie {}

pub use env::Permission;

fn next_id() -> PieToken {
    static NEXT: AtomicUsize = AtomicUsize::new(1);
    PieToken::mint(
        NEXT.try_update(Ordering::Relaxed, Ordering::Relaxed, |id| id.checked_add(1))
            .expect("Pie token identity exhausted"),
    )
}

pub(crate) fn locate(task: &Task, token: PieToken) -> Option<PieSnapshot> {
    let pies = task.pend.pies.lock();
    pies.iter()
        .find(|p| p.token() == token)
        .map(|p| p.snapshot())
}

pub(crate) fn vestor(task: &Arc<Task>, token: PieToken) -> Option<TaskId> {
    for _ in 0..super::RETRIES {
        let observed = {
            let _pend = task.pend.lock();
            locate(task, token)?
        };
        let parent = observed.sire()?;
        let lord = observed.lord().upgrade()?;
        let result = super::with_pair(task, &lord, || {
            let Some(current) = locate(task, token) else {
                return Some(None);
            };
            if current.sire() != Some(parent) || !current.lord().ptr_eq(observed.lord()) {
                return None;
            }
            Some(locate(&lord, parent).map(|_| lord.ident.id))
        });
        if let Some(result) = result {
            return result;
        }
    }
    None
}

pub(crate) fn accede(
    task: &Arc<Task>,
    token: PieToken,
    need: Need,
) -> Result<PieSnapshot, env::PieFail> {
    let pie = locate(task, token).ok_or(env::PieFail::Denied)?;
    if !pie.alive() {
        return Err(env::PieFail::Dead);
    }
    if !pie.allows(need) {
        return Err(env::PieFail::Denied);
    }
    Ok(pie)
}

pub(crate) fn allows(task: &Task, resource: &Mail, need: Need) -> bool {
    let _pend = task.pend.lock();
    task.pend.pies.lock().iter().any(|pie| {
        pie.meta.same(resource) && pie.alive() && pie.heir().is_none() && pie.allows(need)
    })
}
