//! Supervisor publication of Identity's authorized faces.
use ::schedule::{Progress, Res, ResMut};
use crate::system::{
    boot::{Faces, Mounts},
    identity::client::{install::Roster, query},
    run::publication::Internal,
};
use system_client::{identity as id, operator as op};
pub fn faces(
    roster: Res<Roster>,
    faces: Res<Faces>,
    mut mounts: ResMut<Mounts>,
) -> Result<Progress, &'static str> {
    let principal = roster.control().ok_or("Control identity missing")?;
    let authority = query::current_authority(&roster).ok_or("identity authority")?;
    for grant in id::Grant::ALL {
        let permit = match grant.mount() {
            id::Mount::Public => op::Permit::Public,
            id::Mount::Bound => op::Permit::Bound,
            id::Mount::Installer => op::Permit::Identity(id::Selector::Exact(principal)),
        };
        mounts.0.push(Internal {
            road: id::DIR.try_join(grant.name()).ok_or("Identity path")?,
            entry: faces.0[grant.index()],
            access: (permit, authority),
        });
    }
    Ok(Progress::Done)
}
