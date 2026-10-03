use super::super::{
    Binding, CoalitionId, Cursor, Grant, Match, Page, PageTarget, PrincipalId, Reply, Selector,
    Wire,
};
use super::{CallError, Face, flag};
use env::{PieToken, TaskId, Wait};

/// Queries of effective task identity.
pub struct TaskQuery {
    resolve: Face,
    matches: Face,
    same: Face,
}
impl TaskQuery {
    pub fn direct(
        authority: TaskId,
        resolve: PieToken,
        matches: PieToken,
        same: PieToken,
    ) -> Result<Self, CallError> {
        Ok(Self {
            resolve: Face::direct(authority, Grant::Resolve, resolve)?,
            matches: Face::direct(authority, Grant::Matches, matches)?,
            same: Face::direct(authority, Grant::Same, same)?,
        })
    }
    pub fn discover(
        operator: &crate::service::operator::client::Face,
        authority: TaskId,
        wait: Wait,
    ) -> Result<Self, CallError> {
        Ok(Self {
            resolve: Face::discover(operator, authority, Grant::Resolve, wait)?,
            matches: Face::discover(operator, authority, Grant::Matches, wait)?,
            same: Face::discover(operator, authority, Grant::Same, wait)?,
        })
    }
    pub fn authority(&self) -> TaskId {
        self.resolve.authority()
    }
    /// Revalidate the whole source-bound bundle without querying or caching identity state.
    pub fn available(&self) -> bool {
        [&self.resolve, &self.matches, &self.same]
            .into_iter()
            .all(|face| Face::direct(face.authority(), face.grant(), face.entry()).is_ok())
    }

    pub fn resolve(&self, task: TaskId, wait: Wait) -> Result<Option<Binding>, CallError> {
        match self.resolve.call(Wire::Resolve(task), wait)? {
            Reply::Binding(b) => Ok(b),
            _ => Err(CallError::Malformed),
        }
    }
    pub fn matches(
        &self,
        task: TaskId,
        selector: Selector,
        wait: Wait,
    ) -> Result<Match, CallError> {
        match self.matches.call(Wire::Matches(task, selector), wait)? {
            Reply::Match(m) => Ok(m),
            _ => Err(CallError::Malformed),
        }
    }
    pub fn same(&self, a: TaskId, b: TaskId, wait: Wait) -> Result<Match, CallError> {
        match self.same.call(Wire::Same(a, b), wait)? {
            Reply::Match(m) => Ok(m),
            _ => Err(CallError::Malformed),
        }
    }
}

/// The complete eight-operation query capability.
pub struct Query {
    tasks: TaskQuery,
    sire: Face,
    heir: Face,
    amid: Face,
    members: Face,
    memberships: Face,
}
impl Query {
    pub fn discover(
        operator: &crate::service::operator::client::Face,
        authority: TaskId,
        wait: Wait,
    ) -> Result<Self, CallError> {
        Ok(Self {
            tasks: TaskQuery::discover(operator, authority, wait)?,
            sire: Face::discover(operator, authority, Grant::Sire, wait)?,
            heir: Face::discover(operator, authority, Grant::Heir, wait)?,
            amid: Face::discover(operator, authority, Grant::Amid, wait)?,
            members: Face::discover(operator, authority, Grant::Members, wait)?,
            memberships: Face::discover(operator, authority, Grant::Memberships, wait)?,
        })
    }
    pub fn authority(&self) -> TaskId {
        self.tasks.authority()
    }
    pub fn resolve(&self, task: TaskId, wait: Wait) -> Result<Option<Binding>, CallError> {
        self.tasks.resolve(task, wait)
    }
    pub fn matches(
        &self,
        task: TaskId,
        selector: Selector,
        wait: Wait,
    ) -> Result<Match, CallError> {
        self.tasks.matches(task, selector, wait)
    }
    pub fn same(&self, a: TaskId, b: TaskId, wait: Wait) -> Result<Match, CallError> {
        self.tasks.same(a, b, wait)
    }
    pub fn sire(&self, p: PrincipalId, wait: Wait) -> Result<Option<PrincipalId>, CallError> {
        match self.sire.call(Wire::Sire(p), wait)? {
            Reply::Principal(p) => Ok(p),
            _ => Err(CallError::Malformed),
        }
    }
    pub fn heir(&self, a: PrincipalId, b: PrincipalId, wait: Wait) -> Result<bool, CallError> {
        flag(self.heir.call(Wire::Heir(a, b), wait)?)
    }
    pub fn amid(&self, p: PrincipalId, c: CoalitionId, wait: Wait) -> Result<bool, CallError> {
        flag(self.amid.call(Wire::Amid(p, c), wait)?)
    }
    pub fn members(
        &self,
        c: CoalitionId,
        cursor: Option<Cursor>,
        wait: Wait,
    ) -> Result<Page<PrincipalId>, CallError> {
        match self.members.call(Wire::Members(c, cursor), wait)? {
            Reply::Members(p) if p.next().is_none_or(|k| k.target == PageTarget::Members(c)) => {
                Ok(p)
            }
            _ => Err(CallError::Malformed),
        }
    }
    pub fn memberships(
        &self,
        p: PrincipalId,
        cursor: Option<Cursor>,
        wait: Wait,
    ) -> Result<Page<CoalitionId>, CallError> {
        match self.memberships.call(Wire::Memberships(p, cursor), wait)? {
            Reply::Memberships(page)
                if page
                    .next()
                    .is_none_or(|k| k.target == PageTarget::Memberships(p)) =>
            {
                Ok(page)
            }
            _ => Err(CallError::Malformed),
        }
    }
}
