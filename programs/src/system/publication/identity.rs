//! Supervisor publication of Identity's authorized faces.
use crate::system::{
    app::boot::Faces,
    control::identity::{Roster, current_authority},
    publication::Internal,
    publication::Mounts,
};
use ::schedule::{Progress, Res, ResMut};
pub(crate) fn faces(
    roster: Res<Roster>,
    faces: Res<Faces>,
    mut mounts: ResMut<Mounts>,
) -> Result<Progress, &'static str> {
    let principal = roster.control().ok_or("Control identity missing")?;
    let authority = current_authority(&roster).ok_or("identity authority")?;
    for grant in system_api::identity::Grant::ALL {
        let permit = match grant.mount() {
            system_api::identity::Mount::Public => system_api::operator::Permit::Public,
            system_api::identity::Mount::Bound => system_api::operator::Permit::Bound,
            system_api::identity::Mount::Installer => system_api::operator::Permit::Identity(
                system_api::identity::Selector::Exact(principal),
            ),
        };
        mounts.0.push(Internal {
            road: system_api::operator::Path::new(system_api::identity::DIR)
                .try_join(grant.name())
                .ok_or("Identity path")?,
            entry: faces.0[grant.index()],
            access: (permit, authority),
        });
    }
    Ok(Progress::Done)
}
