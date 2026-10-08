use crate::support::machine::Machine;
use crate::system::control::identity::Roster;
use crate::system::control::identity::validate_permit;
use crate::system::control::unit::Control;
use crate::system::operator::Placement;
use crate::system::operator::tree::Tile;
use crate::system::publication::runtime::Resources;
use ::schedule::{Progress, Res, ResMut};
use system_api::control::publication as pubcall;
use system_api::control::publication::Scope;
use system_api::control::publication::Target;
use system_api::identity::Selector;
use system_api::operator::Fail;
use system_api::operator::Permit;
use system_api::operator::path::Path;

use super::{Approved, Decision, Request};
use ::resource::raw::inspect;
pub fn source(
    request: Res<Request>,
    control: Res<Control>,
    mut decision: ResMut<Decision>,
) -> Result<Progress, &'static str> {
    let request = request.0.as_ref().ok_or("publication request")?;
    if request.back.is_none() || request.frame.op != pubcall::PUBLISH {
        return Ok(Progress::Done);
    }
    let target_live = match request.frame.target() {
        Some(Target::RuntimeResource { task, .. }) => control.live(task),
        _ => true,
    };
    if !control.live(request.from)
        || !target_live
        || !matches!(inspect(request.frame.entry), Ok((vestor, owner, _)) if vestor == request.from && owner == request.from)
    {
        *decision = Decision::Failed(Fail::Denied);
    }
    Ok(Progress::Done)
}
pub fn service(
    request: Res<Request>,
    control: Res<Control>,
    mut decision: ResMut<Decision>,
) -> Result<Progress, &'static str> {
    if !matches!(*decision, Decision::Unset) {
        return Ok(Progress::Done);
    }
    let request = request.0.as_ref().ok_or("publication request")?;
    if request.back.is_none() || request.frame.op != pubcall::PUBLISH {
        return Ok(Progress::Done);
    }
    let Some(target @ Target::Service { .. }) = request.frame.target() else {
        return Ok(Progress::Done);
    };
    let Target::Service { scope, group, name } = &target else {
        unreachable!()
    };
    let program = control.find_named_task(request.from).and_then(|row| {
        crate::unit::PROGRAMS
            .iter()
            .copied()
            .find(|p| p.name() == row.name)
    });
    let mark = inspect(request.frame.entry).map(|(_, _, mark)| mark).ok();
    let mut road = None;
    if let (Some(program), Some(mark)) = (program, mark) {
        for rule in program.publication {
            match rule {
                crate::unit::Publish::Devices
                    if *scope == Scope::Device && mark == hub_api::Grant::Claim.mark() =>
                {
                    if matches!(
                        request.frame.permit,
                        Permit::Identity(Selector::MemberOf(_))
                    ) {
                        *decision = Decision::Device;
                    }
                }
                crate::unit::Publish::Entries {
                    scope: allowed,
                    group: expected,
                    road: base,
                    entries,
                    public,
                } => {
                    let allowed = match allowed {
                        crate::unit::PublishScope::Driver => Scope::Driver,
                        crate::unit::PublishScope::Hub => Scope::Hub,
                        crate::unit::PublishScope::Fixture => Scope::Fixture,
                        crate::unit::PublishScope::Terminal => Scope::Terminal,
                    };
                    if *scope == allowed
                        && group == expected
                        && (!*public || request.frame.permit == Permit::Public)
                        && entries
                            .iter()
                            .any(|e| e.name == name && e.mark.is_none_or(|m| m == mark))
                    {
                        road = Path::new(base).try_join(name);
                    }
                }
                _ => {}
            }
        }
    }
    if let Some(road) = road {
        *decision = Decision::Install(Approved {
            target,
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
        });
    } else if !matches!(*decision, Decision::Device) {
        *decision = Decision::Failed(Fail::Denied);
    }
    Ok(Progress::Done)
}
pub fn device(
    request: Res<Request>,
    machine: Res<Machine>,
    mut decision: ResMut<Decision>,
) -> Result<Progress, &'static str> {
    if !matches!(*decision, Decision::Device) {
        return Ok(Progress::Done);
    }
    let request = request.0.as_ref().ok_or("publication request")?;
    let Some(target @ Target::Service { .. }) = request.frame.target() else {
        return Err("publication device target");
    };
    let Target::Service { group, name, .. } = &target else {
        unreachable!()
    };
    let valid = (group == hub_api::BOOT
        && [hub_api::DTB, hub_api::SUPERVISOR_EXTERNAL].contains(&name.as_str()))
        || machine.devices().is_some_and(|devices| {
            devices
                .iter()
                .any(|d| d.class.as_str() == group && d.name.as_str() == name)
        });
    let road = valid
        .then(|| {
            Path::new("dev")
                .try_join(group)
                .and_then(|p| p.try_join(name))
        })
        .flatten();
    *decision = match road {
        Some(road) => Decision::Install(Approved {
            target,
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
            if matches!(
                approved.target,
                Target::Service {
                    scope: Scope::Device,
                    ..
                }
            ) {
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
