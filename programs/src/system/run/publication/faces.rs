use ::schedule::{Progress, Res, ResMut};
use super::Internal;
use crate::system::{
    boot::Mounts,
    common::face::mount,
    control::serve::{start::Images, watch::Watch},
    identity::client::install::Roster,
};
use protocol::{
    common::{
        path::Path,
    },
};
use system_client::{identity as id, operator as op};
pub fn publication_face(
    images: Res<Images>,
    mut mounts: ResMut<Mounts>,
) -> Result<Progress, &'static str> {
    mounts.0.push(Internal {
        road: Path::new("svc/sys/control/publish").to_path_buf(),
        entry: images.entry,
        access: (op::Permit::Public, env::unit::self_id()),
    });
    Ok(Progress::Done)
}
pub fn faces(
    roster: Res<Roster>,
    mut mounts: ResMut<Mounts>,
    mut watch: ResMut<Watch>,
) -> Result<Progress, &'static str> {
    let principal = roster.control().ok_or("Control identity missing")?;
    for grant in system_api::control::Grant::ALL {
        let permit = if grant == system_api::control::Grant::State {
            op::Permit::Public
        } else {
            op::Permit::Identity(id::Selector::Exact(principal))
        };
        let (entry, _) = mount::entry(grant.mark(), grant.name())?;
        mounts.0.push(Internal {
            road: system_api::control::DIR
                .try_join(grant.name())
                .ok_or("Control path")?,
            entry,
            access: (permit, env::unit::self_id()),
        });
        watch.attach_face(grant, entry);
    }
    Ok(Progress::Done)
}
