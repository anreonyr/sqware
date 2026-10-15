use alloc::sync::{Arc, Weak};
use env::{Mark, PieToken, TaskId};

use crate::core::work::grant::Grant;
use crate::core::work::mail::Mail;
use crate::core::work::unit::task::Task;

use super::super::Need;
use super::{Heir, Permission, Pie};

#[derive(Clone)]
pub(crate) struct PieSnapshot {
    pub(super) token: PieToken,
    pub(super) grant: Arc<Grant>,
    pub(super) mark: Mark,
    pub(super) sire: Option<PieToken>,
    pub(super) lord: Weak<Task>,
    pub(super) heir: Option<Heir>,
    pub(super) meta: Mail,
}

impl PieSnapshot {
    /// 这一枚 Pie 的 Grant：持有它的调用方据此知道自己的分支在哪条撤销边上。
    pub(crate) fn grant(&self) -> &Arc<Grant> {
        &self.grant
    }

    pub(crate) fn token(&self) -> PieToken {
        self.token
    }

    pub(crate) fn permission(&self) -> Permission {
        self.grant.permission().unwrap_or_else(Permission::empty)
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

    pub(crate) fn owner(&self) -> Option<TaskId> {
        self.alive().then(|| self.meta.owner())
    }

    pub(crate) fn alive(&self) -> bool {
        self.meta.alive() && self.grant.permission().is_some()
    }

    pub(crate) fn allows(&self, need: Need) -> bool {
        need.allowed_by(self.permission())
    }

    pub(crate) fn mail(&self) -> &Mail {
        &self.meta
    }

    pub(crate) fn vest(&self, mark: Mark, permission: Permission) -> Result<Pie, env::PieFail> {
        Pie::try_new(self.meta.clone(), mark, permission, Some(self.token))
    }
}
