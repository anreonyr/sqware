mod answer;
mod execution;
mod plan;
mod publication;
mod resources;
mod watch;

use ::schedule::{BuildError, Plan, Progress, Res, ResMut, Resources};
use env::PieToken;

pub(crate) fn install(resources: &mut Resources<'static>) -> Result<(), &'static str> {
    resources::resources(resources)
}

pub(crate) fn entry(resources: &Resources<'_>) -> Result<PieToken, &'static str> {
    resources
        .read::<answer::Inbox>()
        .map_err(|_| "Loader inbox is not installed")?
        .entry
        .ok_or("Loader service is not published")
}

pub(crate) fn faces(
    roster: Res<crate::system::control::identity::Roster>,
    mounts: ResMut<crate::system::publication::Mounts>,
    inbox: ResMut<answer::Inbox>,
) -> Result<Progress, &'static str> {
    publication::faces(roster, mounts, inbox)
}

pub(crate) fn frame() -> Result<Plan<crate::system::app::Fault>, BuildError> {
    plan::frame()
}

pub(crate) fn shutdown() -> Result<Plan<crate::system::app::Fault>, BuildError> {
    plan::shutdown()
}

pub(crate) fn watch(
    inbox: Res<answer::Inbox>,
    wanted: ResMut<crate::system::app::wait::Interests>,
) -> Result<Progress, crate::system::app::Fault> {
    watch::entries(inbox, wanted)
}
