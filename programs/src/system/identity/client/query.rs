use super::install::Roster;
use crate::system::common::timing::BOOT_MS;
use env::{TaskId, Wait};
use ipc::session::establish;
use system_client::control::Object;
use system_client::identity::{Grant, Selector, Wire, client::Face};
use system_client::operator::{Fail, Permit};

pub(crate) fn current_authority(roster: &Roster) -> Option<TaskId> {
    roster
        .authority()
        .filter(|authority| !env::unit::join(*authority, Wait::POLL).unwrap_or(true))
}
fn face(roster: &Roster, grant: Grant) -> Result<Face, Fail> {
    let authority = roster.authority().ok_or(Fail::Unjudged)?;
    let entry = establish::find(authority, grant.mark()).ok_or(Fail::Unjudged)?;
    Face::direct(authority, grant, entry).map_err(|_| Fail::Unjudged)
}
pub(crate) fn binding(
    roster: &Roster,
    task: TaskId,
) -> Result<Option<system_client::identity::Binding>, Fail> {
    match face(roster, Grant::Resolve)?
        .call(Wire::Resolve(task), Wait::AtMost(BOOT_MS))
        .map_err(|_| Fail::Unjudged)?
    {
        system_client::identity::Reply::Binding(b) => Ok(b),
        _ => Err(Fail::Unjudged),
    }
}
pub(crate) fn validate(roster: &Roster, object: Object) -> Result<(), Fail> {
    if roster.authority() != Some(object.authority()) {
        return Err(Fail::Unjudged);
    }
    let (grant, wire) = match object {
        Object::Principal(p) => (Grant::Heir, Wire::Heir(p, p)),
        Object::Coalition(c) => (Grant::Members, Wire::Members(c, None)),
    };
    face(roster, grant)?
        .call(wire, Wait::AtMost(BOOT_MS))
        .map(|_| ())
        .map_err(|_| Fail::Unjudged)
}
pub(crate) fn validate_permit(roster: &Roster, permit: Permit) -> Result<(), Fail> {
    match permit {
        Permit::Identity(Selector::Exact(p) | Selector::DescendantOf(p)) => {
            validate(roster, Object::Principal(p))
        }
        Permit::Identity(Selector::MemberOf(c)) => validate(roster, Object::Coalition(c)),
        _ => Ok(()),
    }
}
