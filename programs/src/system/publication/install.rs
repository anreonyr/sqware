use crate::system::operator::management::Tree;
use crate::system::publication::book::{Address, Installation, Publications, Record, Source};
use crate::system::publication::names::Names;
use crate::system::publication::runtime::Resources;
use ::schedule::{Progress, Res, ResMut};
use system_api::control::publication as pubcall;
use system_api::control::publication::Object;
use system_api::control::publication::Reply;
use system_api::control::publication::Target;
use system_api::identity::Selector;
use system_api::operator::EntryId;
use system_api::operator::Fail;
use system_api::operator::Permit;

use super::{Decision, Kind, Outcome, Request};
use ::resource::raw::pies;
use env::pie;
impl Publications {
    pub(super) fn remove(&mut self, tree: &mut Tree, at: usize) -> Result<(), &'static str> {
        let r = &self.records[at];
        if let Some(mount) = r.installation.mount {
            tree.unmount(mount)?;
        }
        self.records[at].installation.mount = None;
        let r = &self.records[at];
        if r.address.target.is_some() {
            let entry = r.source.entry;
            if pies().any(|p| p.token == entry) {
                pie::release(entry, env::ReleaseMode::Keep).map_err(|_| "publication reference cleanup")?;
            }
        }
        self.records.remove(at);
        Ok(())
    }
}
pub fn reset(
    mut decision: ResMut<Decision>,
    mut outcome: ResMut<Outcome>,
    mut kind: ResMut<Kind>,
) -> Result<Progress, &'static str> {
    *decision = Decision::Unset;
    outcome.0 = None;
    kind.road = None;
    Ok(Progress::Done)
}
pub fn simple(
    request: Res<Request>,
    resources: Res<Resources>,
    mut outcome: ResMut<Outcome>,
) -> Result<Progress, &'static str> {
    let request = request.0.as_ref().ok_or("publication request")?;
    if request.back.is_none() {
        return Ok(Progress::Done);
    }
    match (request.frame.op, request.frame.target()) {
        (_, None) => outcome.0 = Some(Err(Fail::Denied)),
        (pubcall::RUNTIME, Some(Target::RuntimeResource { task, .. })) => {
            outcome.0 = Some(resources.team(task).ok_or(Fail::Unknown).map(|team| Reply {
                status: 0,
                kind: 0,
                task,
                number: team.get() as u64,
            }))
        }
        (pubcall::PUBLISH | pubcall::UNPUBLISH, _) => {}
        _ => outcome.0 = Some(Err(Fail::Denied)),
    }
    Ok(Progress::Done)
}
pub fn unpublish(
    request: Res<Request>,
    publications: Res<Publications>,
    mut decision: ResMut<Decision>,
) -> Result<Progress, &'static str> {
    let request = request.0.as_ref().ok_or("publication request")?;
    if request.back.is_none() || request.frame.op != pubcall::UNPUBLISH {
        return Ok(Progress::Done);
    }
    let Some(target) = request.frame.target() else {
        *decision = Decision::Failed(Fail::Denied);
        return Ok(Progress::Done);
    };
    *decision = match publications
        .records
        .iter()
        .position(|r| r.address.target.as_ref() == Some(&target))
    {
        Some(at) if publications.records[at].source.publisher == request.from => {
            Decision::Remove(at)
        }
        Some(_) => Decision::Failed(Fail::Denied),
        None => Decision::Failed(Fail::Unknown),
    };
    Ok(Progress::Done)
}
pub fn withdraw(
    mut decision: ResMut<Decision>,
    mut publications: ResMut<Publications>,
    mut tree: ResMut<Tree>,
) -> Result<Progress, &'static str> {
    if let Decision::Remove(at) = *decision {
        *decision = match publications.remove(&mut tree, at) {
            Ok(()) => Decision::Removed,
            Err(_) => Decision::Failed(Fail::Unknown),
        };
    }
    Ok(Progress::Done)
}
pub fn existing(
    mut publications: ResMut<Publications>,
    mut decision: ResMut<Decision>,
    mut outcome: ResMut<Outcome>,
) -> Result<Progress, &'static str> {
    if let Decision::Install(approved) = &*decision {
        if publications.records.try_reserve(1).is_err() {
            *decision = Decision::Failed(Fail::Full);
            return Ok(Progress::Done);
        }
        if let Some(record) = publications
            .records
            .iter()
            .find(|r| r.address.road == approved.placement.road)
        {
            outcome.0 = Some(
                if record.installation.mount.is_some()
                    && record.source.publisher == approved.publisher
                    && record.address.target.as_ref() == Some(&approved.policy.target)
                    && record.installation.owner == approved.placement.tile.owner.unwrap()
                    && record.source.permit == approved.placement.tile.permit
                    && pie::same(record.source.entry, approved.placement.tile.pie).unwrap_or(false)
                {
                    Ok(Reply::mount(record.installation.mount.unwrap()))
                } else {
                    Err(Fail::Denied)
                },
            );
            *decision = Decision::Unset;
        }
    }
    Ok(Progress::Done)
}
pub fn prepare_kind(
    mut decision: ResMut<Decision>,
    mut resources: ResMut<Resources>,
    mut kind: ResMut<Kind>,
) -> Result<Progress, &'static str> {
    if let Decision::Install(approved) = &*decision {
        if let Target::RuntimeResource {
            task, kind: name, ..
        } = &approved.policy.target
        {
            match resources.prepare_kind(*task, name) {
                Ok(road) => kind.road = road,
                Err(fail) => *decision = Decision::Failed(fail),
            }
        }
    }
    Ok(Progress::Done)
}
pub fn install(
    mut decision: ResMut<Decision>,
    mut tree: ResMut<Tree>,
    kind: Res<Kind>,
) -> Result<Progress, &'static str> {
    let old = core::mem::replace(&mut *decision, Decision::Unset);
    *decision = match old {
        Decision::Install(approved) => match tree.mount(&approved.placement) {
            Ok(mount) => Decision::Mounted(approved, mount),
            Err(_) => {
                if let Some(road) = &kind.road {
                    let _ = tree.remove_empty(road);
                }
                Decision::Failed(Fail::Unknown)
            }
        },
        other => other,
    };
    Ok(Progress::Done)
}
pub fn commit_kind(
    decision: Res<Decision>,
    mut resources: ResMut<Resources>,
    mut kind: ResMut<Kind>,
) -> Result<Progress, &'static str> {
    if let Decision::Mounted(approved, _) = &*decision {
        if let Some(road) = kind.road.take() {
            resources
                .commit_kind(approved.placement.tile.owner.unwrap(), road)
                .map_err(|_| "runtime kind commit")?;
        }
    }
    Ok(Progress::Done)
}
pub fn alias(
    mut decision: ResMut<Decision>,
    mut names: ResMut<Names>,
    mut tree: ResMut<Tree>,
) -> Result<Progress, &'static str> {
    if let Decision::BindAlias { .. } = &*decision {
        let old = core::mem::replace(&mut *decision, Decision::Unset);
        let Decision::BindAlias { registration, .. } = old else {
            unreachable!()
        };
        let name = registration.name.clone();
        let object = registration.object;
        *decision = if names.register(&mut tree, registration).is_ok() {
            names
                .mount_of(&name, object)
                .map(Decision::AliasMounted)
                .unwrap_or(Decision::Failed(Fail::Unknown))
        } else {
            Decision::Failed(Fail::Denied)
        };
    }
    if let Decision::Mounted(approved, mount) = &*decision {
        if let Target::Service { group, .. } = &approved.policy.target
            && approved.policy.alias
        {
            if let Permit::Identity(Selector::MemberOf(c)) = approved.placement.tile.permit {
                if names
                    .register(
                        &mut tree,
                        crate::system::publication::names::Registration {
                            name: group.clone(),
                            object: Object::Coalition(c),
                            lifetime: Some(approved.publisher),
                        },
                    )
                    .is_err()
                {
                    tree.unmount(*mount)?;
                    *decision = Decision::Failed(Fail::Denied);
                }
            }
        }
    }
    Ok(Progress::Done)
}
pub fn commit(
    mut decision: ResMut<Decision>,
    mut publications: ResMut<Publications>,
    mut outcome: ResMut<Outcome>,
) -> Result<Progress, &'static str> {
    let old = core::mem::replace(&mut *decision, Decision::Unset);
    match old {
        Decision::Mounted(approved, mount) => {
            publications.records.push(Record {
                address: Address {
                    road: approved.placement.road,
                    target: Some(approved.policy.target),
                },
                source: Source {
                    publisher: approved.publisher,
                    entry: approved.placement.tile.pie,
                    permit: approved.placement.tile.permit,
                },
                installation: Installation {
                    owner: approved.placement.tile.owner.unwrap(),
                    mount: Some(mount),
                },
            });
            outcome.0 = Some(Ok(Reply::mount(mount)));
        }
        Decision::Failed(fail) => outcome.0 = Some(Err(fail)),
        Decision::AliasMounted(mount) => outcome.0 = Some(Ok(Reply::mount(mount))),
        Decision::Removed => outcome.0 = Some(Ok(Reply::mount(EntryId::new(0)))),
        _ => {}
    }
    Ok(Progress::Done)
}
