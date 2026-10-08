use super::super::{Grant, PrincipalId, Subject, Wire};
use super::{CallError, Face, principal, unit};
use env::{PieToken, TaskId, Wait};

pub struct SelfOps {
    adopt: Face,
    waive: Face,
    restrict: Face,
    derive: Face,
}
impl SelfOps {
    pub fn direct(
        authority: TaskId,
        adopt: PieToken,
        waive: PieToken,
        restrict: PieToken,
        derive: PieToken,
    ) -> Result<Self, CallError> {
        Ok(Self {
            adopt: Face::direct(authority, Grant::Adopt, adopt)?,
            waive: Face::direct(authority, Grant::Waive, waive)?,
            restrict: Face::direct(authority, Grant::Restrict, restrict)?,
            derive: Face::direct(authority, Grant::Derive, derive)?,
        })
    }
    pub fn discover(
        operator: &crate::operator::Face,
        authority: TaskId,
        wait: Wait,
    ) -> Result<Self, CallError> {
        Ok(Self {
            adopt: Face::discover(operator, authority, Grant::Adopt, wait)?,
            waive: Face::discover(operator, authority, Grant::Waive, wait)?,
            restrict: Face::discover(operator, authority, Grant::Restrict, wait)?,
            derive: Face::discover(operator, authority, Grant::Derive, wait)?,
        })
    }
    pub fn adopt(&self, s: Subject, wait: Wait) -> Result<(), CallError> {
        unit(self.adopt.call(Wire::Adopt(s), wait)?)
    }
    pub fn waive(&self, wait: Wait) -> Result<(), CallError> {
        unit(self.waive.call(Wire::Waive, wait)?)
    }
    pub fn restrict(&self, s: Subject, wait: Wait) -> Result<(), CallError> {
        unit(self.restrict.call(Wire::Restrict(s), wait)?)
    }
    pub fn derive(&self, p: PrincipalId, wait: Wait) -> Result<PrincipalId, CallError> {
        principal(self.derive.call(Wire::Derive(p), wait)?)
    }
}
