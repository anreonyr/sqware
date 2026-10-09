use super::Internal;
use crate::support::face::mount;
use crate::system::{
    control::identity::Roster,
    control::Entries as Watch,
    publication::Mounts,
};
use ::schedule::{Progress, Res, ResMut};
use system_api::operator::Path;
pub fn publication_face(
    entry: Res<super::Entry>,
    mut mounts: ResMut<Mounts>,
) -> Result<Progress, &'static str> {
    mounts.0.push(Internal {
        road: Path::new("svc/sys/control/publish").to_path_buf(),
        entry: entry.0,
        access: (system_api::operator::Permit::Public, env::unit::self_id()),
    });
    Ok(Progress::Done)
}
pub(crate) fn faces(
    roster: Res<Roster>,
    mut mounts: ResMut<Mounts>,
    mut watch: ResMut<Watch>,
) -> Result<Progress, &'static str> {
    let principal = roster.control().ok_or("Control identity missing")?;
    for grant in system_api::control::Grant::ALL {
        let permit = if grant == system_api::control::Grant::State {
            system_api::operator::Permit::Public
        } else {
            system_api::operator::Permit::Identity(system_api::identity::Selector::Exact(principal))
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

pub(crate) fn instance_face(
    mut watch: ResMut<crate::system::control::Entries>,
    mut mounts: ResMut<crate::system::publication::Mounts>,
) -> Result<Progress, &'static str> {
    let entry =
        env::pie::unseal(env::UnsealArgs::hole(system_api::control::ASK_MARK)).map_err(|_| "instance entry")?;
    watch.attach_instance(entry);
    mounts.0.push(crate::system::publication::Internal {
        road: system_api::control::INSTANCE.to_path_buf(),
        entry,
        access: (system_api::operator::Permit::Bound, env::unit::self_id()),
    });
    Ok(Progress::Done)
}
