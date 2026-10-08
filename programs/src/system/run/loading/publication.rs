use super::answer::Inbox;
use crate::system::{
    boot::Mounts,
    common::face::mount,
    control::Fail,
    control::identity::Roster,
    operator::client::Tree,
    run::publication::{Internal, book::Publications},
};
use ::schedule::{Progress, Res, ResMut};
pub(crate) fn faces(
    roster: Res<Roster>,
    mut mounts: ResMut<Mounts>,
    mut inbox: ResMut<Inbox>,
) -> Result<Progress, &'static str> {
    let principal = roster.control().ok_or("Control identity missing")?;
    let grant = system_api::loader::Grant::Build;
    let (entry, _) = mount::entry(grant.mark(), grant.name())?;
    mounts.0.push(Internal {
        road: system_api::operator::Path::new(system_api::loader::DIR)
            .try_join(grant.name())
            .ok_or("Loader path")?,
        entry,
        access: (
            system_api::operator::Permit::Identity(system_api::identity::Selector::Exact(
                principal,
            )),
            env::unit::self_id(),
        ),
    });
    inbox.entry = Some(entry);
    Ok(Progress::Done)
}

pub fn withdraw(
    inbox: Res<Inbox>,
    mut publications: ResMut<Publications>,
    mut tree: ResMut<Tree>,
) -> Result<Progress, Fail> {
    if let Some(entry) = inbox.entry {
        publications
            .withdraw_internal(&mut tree, entry)
            .map_err(|_| Fail::Publication)?;
    }
    Ok(Progress::Done)
}
