use core::ops::Deref;
use core::sync::atomic::{AtomicUsize, Ordering};

use alloc::sync::{Arc, Weak};

use env::{Mark, PieToken, TaskId};

use crate::work::grant::Grant;
use crate::work::mail::{HoleMeta, Mail, NoleMeta, PoleMeta, ToleMeta};
use crate::work::unit::task::Task;

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
        let token = alloc_id();
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

    pub(crate) fn set_permission(&mut self, subset: Permission) -> Result<(), env::PieFail> {
        self.core.grant.narrow(subset)
    }

    pub(crate) fn narrow(&mut self, subset: Permission) -> Result<(), env::PieFail> {
        if !self.core.alive() {
            return Err(env::PieFail::Dead);
        }
        let permission = self.core.permission();
        if subset.is_empty()
            || !permission.contains(subset)
            || permission.contains(Permission::ONLY) && !subset.contains(Permission::ONLY)
        {
            return Err(env::PieFail::Denied);
        }
        self.set_permission(subset)
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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Heir {
    pub(crate) task: TaskId,
    pub(crate) token: PieToken,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Need {
    Fetch,
    Store,
    Vest,
}

fn alloc_id() -> PieToken {
    static NEXT_ID: AtomicUsize = AtomicUsize::new(1);
    PieToken::mint(
        NEXT_ID
            .try_update(Ordering::Relaxed, Ordering::Relaxed, |id| id.checked_add(1))
            .expect("Pie token identity exhausted"),
    )
}

pub(crate) fn form_ok(src: Permission, subset: Permission) -> bool {
    src.contains(Permission::ONLY) == subset.contains(Permission::ONLY)
}

#[derive(Clone)]
pub(crate) struct PieSnapshot {
    pub(crate) token: PieToken,
    grant: Arc<Grant>,
    mark: Mark,
    pub(crate) sire: Option<PieToken>,
    lord: Weak<Task>,
    heir: Option<Heir>,
    meta: Mail,
}

impl PieSnapshot {
    pub(crate) fn grant(&self) -> &Arc<Grant> {
        &self.grant
    }

    pub(crate) fn token(&self) -> PieToken {
        self.token
    }

    pub(crate) fn permission(&self) -> Permission {
        self.grant.permission().unwrap_or_else(Permission::empty)
    }

    pub(crate) fn kind(&self) -> env::PieKind {
        self.meta.kind()
    }

    pub(crate) fn sire(&self) -> Option<PieToken> {
        self.sire
    }

    pub(crate) fn lord(&self) -> &Weak<Task> {
        &self.lord
    }

    pub(crate) fn heir(&self) -> Option<&Heir> {
        self.heir.as_ref()
    }

    pub(crate) fn mark(&self) -> Mark {
        self.mark
    }

    pub(crate) fn owner_task(&self) -> TaskId {
        self.meta.owner()
    }

    pub(crate) fn owner(&self) -> Option<TaskId> {
        self.alive().then(|| self.owner_task())
    }

    pub(crate) fn alive(&self) -> bool {
        self.meta.alive() && self.grant.permission().is_some()
    }

    pub(crate) fn allows(&self, need: Need) -> bool {
        allows_permission(self.permission(), need)
    }

    pub(crate) fn covers(&self, subset: Permission) -> bool {
        self.permission().contains(subset) && !subset.is_empty()
    }

    pub(crate) fn hole(&self) -> Option<Arc<HoleMeta>> {
        self.meta.hole()
    }

    pub(crate) fn pole(&self) -> Option<Arc<PoleMeta>> {
        self.meta.pole()
    }

    pub(crate) fn nole(&self) -> Option<Arc<NoleMeta>> {
        self.meta.nole()
    }

    pub(crate) fn tole(&self) -> Option<Arc<ToleMeta>> {
        self.meta.tole()
    }

    pub(crate) fn seal(&self) {
        self.meta.seal();
    }

    pub(crate) fn same(&self, other: &Self) -> bool {
        self.meta.same(&other.meta)
    }

    pub(crate) fn vest(&self, mark: Mark, permission: Permission) -> Result<Pie, env::PieFail> {
        Pie::try_new(self.meta.clone(), mark, permission, Some(self.token))
    }
}

fn allows_permission(permission: Permission, need: Need) -> bool {
    permission.contains(match need {
        Need::Fetch => Permission::FETCH,
        Need::Store => Permission::STORE,
        Need::Vest => Permission::VEST,
    })
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
