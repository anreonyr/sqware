use super::super::{Grant, IdentityName, IdentityObject, Install, InstallId, InstallSpec, InstallStatus, OperatorEntryRef, PrincipalId, Reply, Wire};
use super::{CallError, Face, principal, unit};
use env::{PieToken, TaskId, Wait};

pub struct Installer {
    bind: Option<Face>,
    unbind: Option<Face>,
    derive: Face,
    install: Option<Face>, inspect: Option<Face>, cancel: Option<Face>, retire: Option<Face>,
}
impl Installer {
    pub fn authority(&self) -> TaskId {
        self.install.as_ref().or(self.bind.as_ref()).expect("installer has an authority face").authority()
    }
    pub fn direct(
        authority: TaskId,
        bind: PieToken,
        unbind: PieToken,
        derive: PieToken,
    ) -> Result<Self, CallError> {
        Ok(Self {
            bind: Some(Face::direct(authority, Grant::Bind, bind)?),
            unbind: Some(Face::direct(authority, Grant::Unbind, unbind)?),
            derive: Face::direct(authority, Grant::Derive, derive)?,
            install: None, inspect: None, cancel: None, retire: None,
        })
    }
    pub fn direct_with_install(authority: TaskId, install: PieToken, derive: PieToken) -> Result<Self, CallError> {
        Ok(Self {
            bind: None,
            unbind: None,
            derive: Face::direct(authority, Grant::Derive, derive)?,
            install: Some(Face::direct(authority, Grant::Install, install)?),
            inspect: Some(Face::direct(authority, Grant::Install, install)?),
            cancel: Some(Face::direct(authority, Grant::Install, install)?),
            retire: Some(Face::direct(authority, Grant::Install, install)?),
        })
    }
    pub fn install(&self, id: InstallId, spec: InstallSpec, wait: Wait) -> Result<InstallStatus, CallError> { installation(self.install.as_ref().ok_or(CallError::Malformed)?, Wire::Install(id, spec), wait) }
    pub fn inspect(&self, id: InstallId, wait: Wait) -> Result<InstallStatus, CallError> { installation(self.inspect.as_ref().ok_or(CallError::Malformed)?, Wire::Inspect(id), wait) }
    pub fn cancel(&self, id: InstallId, wait: Wait) -> Result<InstallStatus, CallError> { installation(self.cancel.as_ref().ok_or(CallError::Malformed)?, Wire::Cancel(id), wait) }
    pub fn retire(&self, id: InstallId, wait: Wait) -> Result<InstallStatus, CallError> { installation(self.retire.as_ref().ok_or(CallError::Malformed)?, Wire::Retire(id), wait) }
    pub fn associate(&self, object: IdentityObject, name: IdentityName, entry: OperatorEntryRef, wait: Wait) -> Result<(), CallError> {
        unit(self.install.as_ref().ok_or(CallError::Malformed)?.call(Wire::Associate(object, name, entry), wait)?)
    }
    pub fn bind(&self, task: TaskId, install: Install, wait: Wait) -> Result<(), CallError> {
        unit(self.bind.as_ref().ok_or(CallError::Malformed)?.call(Wire::Bind(task, install), wait)?)
    }
    pub fn unbind(&self, task: TaskId, wait: Wait) -> Result<(), CallError> {
        unit(self.unbind.as_ref().ok_or(CallError::Malformed)?.call(Wire::Unbind(task), wait)?)
    }
    pub fn derive(&self, parent: PrincipalId, wait: Wait) -> Result<PrincipalId, CallError> {
        principal(self.derive.call(Wire::Derive(parent), wait)?)
    }
}
fn installation(face: &Face, wire: Wire, wait: Wait) -> Result<InstallStatus, CallError> {
    match face.call(wire, wait)? { Reply::Installation(status) => Ok(status), _ => Err(CallError::Malformed) }
}
