use crate::system::control::identity::Roster;
use crate::system::control::identity::validate_permit;
use crate::system::control::unit::Control;
use crate::system::operator::Placement;
use crate::system::operator::tree::Tile;
use crate::system::publication::runtime::Resources;
use ::schedule::{Progress, Res, ResMut};
use system_api::control::publication as pubcall;
use system_api::control::publication::Target;
use system_api::identity::Selector;
use system_api::operator::Fail;
use system_api::operator::Permit;

use super::{Approved, Decision, Request};
use ::resource::raw::inspect;
pub fn source(
    mut request: ResMut<Request>,
    control: Res<Control>,
    mut decision: ResMut<Decision>,
) -> Result<Progress, &'static str> {
    request.1 = request.0.as_ref().and_then(|incoming| {
        control
            .find_named_task(incoming.from)
            .map(|row| row.name.clone())
    });
    let request = request.0.as_ref().ok_or("publication request")?;
    if request.back.is_none() || request.frame.op != pubcall::PUBLISH {
        return Ok(Progress::Done);
    }
    if matches!(request.frame.target(), Some(Target::IdentityName { .. })) {
        if !control.live(request.from)
            || request.frame.entry != env::PieToken::NONE
            || request.frame.permit != Permit::Bound
        {
            *decision = Decision::Failed(Fail::Denied);
        }
        return Ok(Progress::Done);
    }
    let target_live = match request.frame.target() {
        Some(Target::RuntimeResource { task, .. }) => control.live(task),
        _ => true,
    };
    if !control.live(request.from)
        || !target_live
        || !matches!(inspect(request.frame.entry), Ok(info) if info.alive && info.vestor == request.from && info.owner == request.from)
    {
        *decision = Decision::Failed(Fail::Denied);
    }
    Ok(Progress::Done)
}
pub fn service(
    request: Res<Request>,
    mut decision: ResMut<Decision>,
    namespaces: Res<super::Namespaces>,
) -> Result<Progress, &'static str> {
    if !matches!(*decision, Decision::Unset) {
        return Ok(Progress::Done);
    }
    let owner = request.1.as_deref();
    let request = request.0.as_ref().ok_or("publication request")?;
    if request.back.is_none() || request.frame.op != pubcall::PUBLISH {
        return Ok(Progress::Done);
    }
    let Some(target @ Target::Service { .. }) = request.frame.target() else {
        return Ok(Progress::Done);
    };
    let approval =
        owner.and_then(|owner| namespaces.service(owner, (&target, request.frame.permit)));
    *decision = match approval {
        Some((road, (member, alias))) => Decision::Install(Approved {
            policy: super::Approval {
                target,
                member,
                alias,
            },
            placement: Placement {
                road,
                tile: Tile {
                    pie: request.frame.entry,
                    permit: request.frame.permit,
                    owner: Some(request.from),
                },
                replace: false,
            },
            publisher: request.from,
        }),
        None => Decision::Failed(Fail::Denied),
    };
    Ok(Progress::Done)
}
pub(crate) fn alias(
    request: Res<Request>,
    control: Res<Control>,
    mut decision: ResMut<Decision>,
) -> Result<Progress, &'static str> {
    if !matches!(*decision, Decision::Unset) {
        return Ok(Progress::Done);
    }
    let Some(incoming) = &request.0 else {
        return Ok(Progress::Done);
    };
    let Some(Target::IdentityName { object, name }) = incoming.frame.target() else {
        return Ok(Progress::Done);
    };
    if incoming.frame.op != pubcall::PUBLISH {
        return Ok(Progress::Done);
    }
    let permitted = request
        .1
        .as_deref()
        .and_then(|name| control.input(name).ok())
        .is_some_and(|input| input.program.identity.aliases.contains(&name.as_str()));
    *decision = if permitted
        && pubcall::valid_name(&name)
        && matches!(object, pubcall::Object::Principal(_))
    {
        Decision::BindAlias {
            publisher: incoming.from,
            registration: super::names::Registration {
                name,
                object,
                lifetime: Some(incoming.from),
            },
        }
    } else {
        Decision::Failed(Fail::Denied)
    };
    Ok(Progress::Done)
}
pub fn runtime(
    request: Res<Request>,
    resources: Res<Resources>,
    mut decision: ResMut<Decision>,
) -> Result<Progress, &'static str> {
    if !matches!(*decision, Decision::Unset) {
        return Ok(Progress::Done);
    }
    let request = request.0.as_ref().ok_or("publication request")?;
    if request.back.is_none() || request.frame.op != pubcall::PUBLISH {
        return Ok(Progress::Done);
    }
    *decision = resources.policy(request).unwrap_or_else(Decision::Failed);
    Ok(Progress::Done)
}
pub(crate) fn identity(
    roster: Res<Roster>,
    mut decision: ResMut<Decision>,
) -> Result<Progress, &'static str> {
    let mut old = core::mem::replace(&mut *decision, Decision::Unset);
    let validation = match &old {
        Decision::Install(approved) | Decision::OwnHole(approved) => {
            let permit = approved.placement.tile.permit;
            let mut result = validate_permit(&roster, permit);
            if matches!(&old, Decision::OwnHole(_)) {
                result = result.and_then(|_| {
                    let principal =
                        crate::system::control::identity::binding(&roster, approved.publisher)?
                            .ok_or(Fail::Denied)?
                            .current
                            .principal;
                    if permit == Permit::Identity(Selector::Exact(principal)) {
                        Ok(())
                    } else {
                        Err(Fail::Denied)
                    }
                });
            }
            if approved.policy.member {
                result = result.and_then(|_| {
                    let Permit::Identity(Selector::MemberOf(coalition)) = permit else {
                        return Err(Fail::Denied);
                    };
                    let subject =
                        crate::system::control::identity::binding(&roster, approved.publisher)?
                            .ok_or(Fail::Denied)?
                            .current;
                    if subject.coalitions.contains(coalition) {
                        Ok(())
                    } else {
                        Err(Fail::Denied)
                    }
                });
            }
            result
        }
        Decision::BindAlias {
            publisher,
            registration,
        } => match registration.object {
            pubcall::Object::Principal(principal) => {
                system_api::identity::Subject::new(principal, &[])
                    .map_err(|_| Fail::Denied)
                    .and_then(|subject| {
                        roster
                            .allow_subject(*publisher, subject)
                            .map_err(|_| Fail::Denied)
                    })
            }
            _ => Err(Fail::Denied),
        },
        _ => Ok(()),
    };
    if let Err(fail) = validation {
        old = Decision::Failed(fail);
    }
    if let Decision::OwnHole(approved) = old {
        old = Decision::Install(approved);
    }
    *decision = old;
    Ok(Progress::Done)
}
