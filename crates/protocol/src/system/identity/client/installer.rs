use super::super::{Grant, Install, PrincipalId, Wire};
use super::{CallError, Face, principal, unit};
use env::{PieToken, TaskId, Wait};

pub struct Installer {
    bind: Face,
    unbind: Face,
    derive: Face,
}
impl Installer {
    pub fn authority(&self) -> TaskId {
        self.bind.authority()
    }
    pub fn direct(
        authority: TaskId,
        bind: PieToken,
        unbind: PieToken,
        derive: PieToken,
    ) -> Result<Self, CallError> {
        Ok(Self {
            bind: Face::direct(authority, Grant::Bind, bind)?,
            unbind: Face::direct(authority, Grant::Unbind, unbind)?,
            derive: Face::direct(authority, Grant::Derive, derive)?,
        })
    }
    pub fn bind(&self, task: TaskId, install: Install, wait: Wait) -> Result<(), CallError> {
        unit(self.bind.call(Wire::Bind(task, install), wait)?)
    }
    pub fn unbind(&self, task: TaskId, wait: Wait) -> Result<(), CallError> {
        unit(self.unbind.call(Wire::Unbind(task), wait)?)
    }
    pub fn derive(&self, parent: PrincipalId, wait: Wait) -> Result<PrincipalId, CallError> {
        principal(self.derive.call(Wire::Derive(parent), wait)?)
    }
}
