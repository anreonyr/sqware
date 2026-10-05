use runtime::schedule::{Progress, Res, ResMut};
use super::answer::Inbox;
use crate::system::{
    boot::Mounts,
    common::face::mount,
    control::serve::Fail,
    identity::client::install::Roster,
    operator::client::Tree,
    run::publication::{Internal, book::Publications},
};
use protocol::{
    system::{identity, loader as call, operator},
};
pub fn faces(
    roster: Res<Roster>,
    mut mounts: ResMut<Mounts>,
    mut inbox: ResMut<Inbox>,
) -> Result<Progress, &'static str> {
    let principal = roster.control().ok_or("Control identity missing")?;
    let grant = call::Grant::Build;
    let (entry, _) = mount::entry(grant.mark(), grant.name())?;
    mounts.0.push(Internal {
        road: call::DIR.try_join(grant.name()).ok_or("Loader path")?,
        entry,
        access: (
            operator::Permit::Identity(identity::Selector::Exact(principal)),
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
