//! Capability-granted creation admission; Identity validates the allowed subject subtree.
use ::schedule::{Progress, Res, ResMut};
use alloc::vec::Vec;
use env::{PieToken, TaskId, Wait};
use ipc::rpc;
use system_api::{
    control::{Fail, construction as api},
    loader::Said,
};
pub(crate) struct Incoming {
    provenance: NativeProvenance,
    call: ReceivedCall,
}
#[derive(Clone, Copy)]
struct NativeProvenance {
    entry: PieToken,
    from: TaskId,
}
struct ReceivedCall {
    request: api::Request,
    back: rpc::reply::Sender<Said>,
}
pub(crate) struct Construction {
    grants: Vec<ConstructionGrant>,
    queued: Vec<(Incoming, Option<crate::unit::Construction>)>,
}
struct ConstructionGrant {
    entry: PieToken,
    task: TaskId,
    policy: crate::unit::Construction,
    delivered: bool,
    revoked: bool,
}
impl Construction {
    pub(crate) fn new() -> env::PieResult<Self> {
        Ok(Self {
            grants: Vec::new(),
            queued: Vec::new(),
        })
    }
    pub(crate) fn entries(&self) -> impl Iterator<Item = PieToken> + '_ {
        self.grants.iter().map(|grant| grant.entry)
    }

    pub(crate) fn retire(
        &mut self,
        task: TaskId,
        mut close_scope: impl FnMut(PieToken, TaskId) -> Result<(), ()>,
    ) -> Result<(), &'static str> {
        let mut index = 0;
        while index < self.grants.len() {
            if self.grants[index].task != task {
                index += 1;
                continue;
            }
            let grant = &mut self.grants[index];
            if !grant.revoked {
                resource::raw::release(grant.entry)
                    .map_err(|_| "construction capability revoke")?;
                grant.revoked = true;
            }
            close_scope(grant.entry, grant.task).map_err(|_| "construction create scope retire")?;
            self.grants.remove(index);
        }
        Ok(())
    }
}
pub(crate) fn receive(
    mut inbox: ResMut<Construction>,
) -> Result<Progress, crate::system::app::Fault> {
    use wire::Message;
    let mut bytes = api::Request::EMPTY;
    let entries = inbox
        .grants
        .iter()
        .map(|grant| grant.entry)
        .collect::<Vec<_>>();
    for entry in entries {
        let receiver =
            rpc::request::Receiver::<api::Call>::from_raw(entry, api::Call::BACK, api::Call::back);
        for _ in 0..16 {
            let incoming = match receiver.receive(&mut bytes, Wait::POLL) {
                Ok(incoming) => incoming,
                Err(rejected) if matches!(rejected.fail, rpc::Fail::Receive(_)) => break,
                Err(rejected) => {
                    if let Some((from, request)) = rejected.incoming {
                        release_input(&request, from);
                    }
                    continue;
                }
            };
            if inbox.queued.len() >= 16 || inbox.queued.try_reserve(1).is_err() {
                release_input(&incoming.request, incoming.from);
                let _ = incoming
                    .reply
                    .send(system_api::loader::Said::from_result(Err(Fail::Full)));
            } else {
                inbox.queued.push((
                    Incoming {
                        provenance: NativeProvenance {
                            entry,
                            from: incoming.from,
                        },
                        call: ReceivedCall {
                            request: incoming.request,
                            back: incoming.reply,
                        },
                    },
                    None,
                ));
            }
        }
    }
    Ok(Progress::Done)
}
pub(crate) fn admit(
    mut inbox: ResMut<Construction>,
    control: Res<crate::system::control::unit::Control>,
    roster: Res<crate::system::control::identity::Roster>,
) -> Result<Progress, crate::system::app::Fault> {
    let mut index = 0;
    while index < inbox.queued.len() {
        let item = &inbox.queued[index].0;
        let build = item.call.request.op == api::BUILD && item.call.request.id.value != 0;
        let query = matches!(item.call.request.op, api::INSPECT | api::CANCEL)
            && item.call.request.id.value != 0
            && item.call.request.input.0.is_none();
        let policy = inbox
            .grants
            .iter()
            .find(|grant| {
                grant.entry == item.provenance.entry
                    && grant.task == item.provenance.from
                    && grant.delivered
                    && !grant.revoked
            })
            .map(|grant| grant.policy);
        let selected = policy.filter(|policy| {
            !build
                || admits(
                    *policy,
                    item,
                    Authority {
                        control: &control,
                        identity: &roster,
                    },
                )
        });
        let allowed = (build && selected.is_some() || query && policy.is_some())
            && control.live(item.provenance.from);
        let failure = if !build && !query {
            Some(Fail::Bad)
        } else if !allowed {
            Some(Fail::Denied)
        } else {
            None
        };
        if let Some(fail) = failure {
            let (item, _) = inbox.queued.remove(index);
            release_input(&item.call.request, item.provenance.from);
            let _ = item
                .call
                .back
                .send(system_api::loader::Said::from_result(Err(fail)));
        } else {
            inbox.queued[index].1 = selected;
            index += 1;
        }
    }
    Ok(Progress::Done)
}

impl Construction {
    pub(crate) fn grant(
        &mut self,
        task: TaskId,
        policies: &'static [crate::unit::Construction],
    ) -> Result<(), &'static str> {
        if policies.is_empty() || policies.iter().any(|policy| !policy.valid()) {
            return if policies.is_empty() {
                Ok(())
            } else {
                Err("invalid construction policy")
            };
        }
        let missing = policies
            .iter()
            .enumerate()
            .filter(|(index, policy)| {
                !policies[..*index].contains(policy)
                    && !self
                        .grants
                        .iter()
                        .any(|grant| grant.task == task && grant.policy == **policy)
            })
            .count();
        if self.grants.len().saturating_add(missing) > 256 {
            return Err("construction grant capacity");
        }
        self.grants
            .try_reserve(missing)
            .map_err(|_| "constructor grant capacity")?;
        for policy in policies.iter().copied() {
            let index = if let Some(index) = self
                .grants
                .iter()
                .position(|grant| grant.task == task && grant.policy == policy)
            {
                if self.grants[index].delivered && !self.grants[index].revoked {
                    continue;
                }
                if self.grants[index].revoked {
                    return Err("constructor grant retired");
                }
                index
            } else {
                let entry = env::hole::unseal(api::ENTRY, env::HoleLimits::default())
                    .map_err(|_| "constructor entry")?;
                self.grants.push(ConstructionGrant {
                    entry,
                    task,
                    policy,
                    delivered: false,
                    revoked: false,
                });
                self.grants.len() - 1
            };
            let grant = &mut self.grants[index];
            if let Err(error) = deliver_grant(grant) {
                if resource::raw::release(grant.entry).is_ok() {
                    self.grants.remove(index);
                }
                return Err(error);
            }
        }
        Ok(())
    }
    pub(crate) fn drain(
        &mut self,
    ) -> impl Iterator<
        Item = (
            PieToken,
            api::Request,
            TaskId,
            rpc::reply::Sender<Said>,
            Option<crate::unit::Construction>,
        ),
    > + '_ {
        self.queued.drain(..).map(|(item, policy)| {
            (
                item.provenance.entry,
                item.call.request,
                item.provenance.from,
                item.call.back,
                policy,
            )
        })
    }
}

fn deliver_grant(grant: &mut ConstructionGrant) -> Result<(), &'static str> {
    let info = resource::raw::inspect(grant.entry).map_err(|_| "constructor inspection")?;
    if !info.alive
        || info.owner != env::unit::self_id()
        || info.vestor != TaskId::new(0)
        || info.kind != env::PieKind::Hole
        || info.mark != api::ENTRY
        || !info.permission.contains(env::Permission::STORE)
    {
        return Err("constructor provenance");
    }
    resource::port::ship(
        grant.entry,
        grant.task,
        env::Access::STORE,
        env::Policy::NONE,
    )
    .map_err(|_| "constructor grant")?;
    grant.delivered = true;
    Ok(())
}

fn release_input(request: &api::Request, from: TaskId) {
    if let Some(api::Input::Image(image)) = &request.input.0 {
        if matches!(resource::raw::inspect(image.image), Ok(info)
            if info.alive && info.vestor == from && info.mark == system_api::loader::IMAGE)
        {
            let _ = env::pie::release(image.image, env::ReleaseMode::Revoke);
        }
    }
}

struct Authority<'a> {
    control: &'a crate::system::control::unit::Control,
    identity: &'a crate::system::control::identity::Roster,
}

fn admits(policy: crate::unit::Construction, item: &Incoming, authority: Authority<'_>) -> bool {
    let Authority {
        control,
        identity: roster,
    } = authority;
    use crate::unit::{ConstructionIdentity, ConstructionOwner, ConstructionSelector};
    use system_api::control::construction::Input;
    let (stack, count, selected) = match (&policy.selector, &item.call.request.input.0) {
        (ConstructionSelector::Unit(expected), Some(Input::Unit(unit))) => (
            unit.stack,
            unit.count,
            *expected == unit.name && unit.name.len() <= 31,
        ),
        (ConstructionSelector::Image, Some(Input::Image(image))) => {
            let capability = resource::raw::inspect(image.image).is_ok_and(|info| {
                info.alive
                    && info.vestor == item.provenance.from
                    && info.mark == system_api::loader::IMAGE
                    && info.kind == env::PieKind::Pole
                    && info.permission.contains(env::Permission::FETCH)
            });
            (
                image.stack,
                image.count,
                capability && image.len > 0 && image.len <= policy.max_image as u64,
            )
        }
        _ => return false,
    };
    if !selected
        || count < policy.arguments.min
        || count > policy.arguments.max
        || stack < policy.stack.min
        || stack > policy.stack.max
        || !control.live(item.call.request.owner)
    {
        return false;
    }
    let subject_ok = match policy.identity {
        ConstructionIdentity::Caller => {
            crate::system::control::identity::binding(roster, item.provenance.from)
                .ok()
                .flatten()
                .is_some_and(|binding| {
                    binding.current.principal == item.call.request.subject.principal
                        && binding.current.coalitions == item.call.request.subject.coalitions
                })
        }
        ConstructionIdentity::Selected => roster
            .allow_subject(item.provenance.from, item.call.request.subject)
            .is_ok(),
        ConstructionIdentity::Exact(subject) => {
            item.call.request.subject == subject
                && roster.allow_subject(item.provenance.from, subject).is_ok()
        }
    };
    let owner_ok = match policy.owner {
        ConstructionOwner::Caller => item.call.request.owner == item.provenance.from,
        ConstructionOwner::Selected => roster
            .allow_subject(item.call.request.owner, item.call.request.subject)
            .is_ok(),
        ConstructionOwner::Exact(owner) => item.call.request.owner == owner,
        ConstructionOwner::NamedUnit(name) => control
            .named_task(name)
            .is_ok_and(|task| task == item.call.request.owner),
    };
    subject_ok && owner_ok
}
