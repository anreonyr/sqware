use super::super::{CoalitionId, Grant, PrincipalId, Reply, Wire};
use super::{CallError, Face, unit};
use env::{PieToken, TaskId, Wait};

pub struct Organization {
    found: Face,
    admit: Face,
    expel: Face,
}
impl Organization {
    pub fn direct(
        authority: TaskId,
        found: PieToken,
        admit: PieToken,
        expel: PieToken,
    ) -> Result<Self, CallError> {
        Ok(Self {
            found: Face::direct(authority, Grant::Found, found)?,
            admit: Face::direct(authority, Grant::Admit, admit)?,
            expel: Face::direct(authority, Grant::Expel, expel)?,
        })
    }
    pub fn discover(
        operator: &crate::operator::Face,
        authority: TaskId,
        wait: Wait,
    ) -> Result<Self, CallError> {
        Ok(Self {
            found: Face::discover(operator, authority, Grant::Found, wait)?,
            admit: Face::discover(operator, authority, Grant::Admit, wait)?,
            expel: Face::discover(operator, authority, Grant::Expel, wait)?,
        })
    }
    pub fn found(&self, wait: Wait) -> Result<CoalitionId, CallError> {
        match self.found.call(Wire::Found, wait)? {
            Reply::Coalition(c) => Ok(c),
            _ => Err(CallError::Malformed),
        }
    }
    pub fn admit(&self, c: CoalitionId, p: PrincipalId, wait: Wait) -> Result<(), CallError> {
        unit(self.admit.call(Wire::Admit(c, p), wait)?)
    }
    pub fn expel(&self, c: CoalitionId, p: PrincipalId, wait: Wait) -> Result<(), CallError> {
        unit(self.expel.call(Wire::Expel(c, p), wait)?)
    }
}
