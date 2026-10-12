//! System-owned export admission for declared Operator bindings.

use alloc::vec::Vec;
use env::{Access, PieToken, TaskId, Wait};
use ipc::rpc;
use system_api::identity::OperatorEntryRef;
use system_api::identity::export as api;
use system_client::operator::{Capability, GrantInfo};

#[path = "export_admission.rs"]
mod admission;

const EXPORT_CAP: usize = 256;

pub struct GrantRequest<'a> {
    pub task: TaskId,
    pub program: &'static crate::unit::UnitFile,
    pub namespaces: &'a crate::system::assembly::namespace::NamespaceInstallations,
}

struct Admission<'a> {
    context: AdmissionContext<'a>,
    grant: &'a ExportGrant,
    caller: CallerRequest,
}

struct AdmissionContext<'a> {
    roster: &'a crate::system::control::identity::Roster,
    control: &'a crate::system::control::unit::Control,
}

struct CallerRequest {
    from: TaskId,
    request: api::Request,
}

struct NamespaceGrant {
    declaration: crate::unit::operator::Namespace,
    grant: GrantInfo,
}

struct ExportOwner {
    task: TaskId,
    program: &'static crate::unit::UnitFile,
}

struct ExportGrant {
    owner: ExportOwner,
    endpoint: PieToken,
    namespaces: Vec<NamespaceGrant>,
    delivered: bool,
}

struct AdmissionTrust<'a> {
    roster: &'a crate::system::control::identity::Roster,
    from: TaskId,
}

struct AdmissionBinding<'a> {
    request: api::Request,
    declared: &'a crate::unit::operator::Binding,
}

struct ActualLookup {
    located: system_api::operator::Binding,
    actual: system_api::operator::BoundResource,
}

struct ActualAdmission<'a> {
    trust: AdmissionTrust<'a>,
    requested: AdmissionBinding<'a>,
    lookup: ActualLookup,
}

/// One scoped export endpoint per installed unit with identity aliases.
#[derive(Default)]
pub struct Exports {
    grants: Vec<ExportGrant>,
    /// The System-local root authority is the only capability used for actual lookups.
    root: Option<Capability>,
    root_entry: Option<system_api::operator::Entry>,
}

impl Exports {
    pub fn bootstrap(
        &mut self,
        system: &crate::system::app::namespace::System,
    ) -> Result<(), &'static str> {
        if self.root.is_some() {
            return Ok(());
        }
        let root = system.shared().root;
        let capability = Capability::direct_at(root.operator, system.capability().entrance(), root)
            .map_err(|_| "identity export root capability")?;
        self.root = Some(capability);
        self.root_entry = Some(root);
        Ok(())
    }

    /// Retains the endpoint receipt before attempting its STORE-only delivery.
    pub fn grant(&mut self, input: GrantRequest<'_>) -> Result<(), &'static str> {
        let GrantRequest {
            task,
            program,
            namespaces,
        } = input;
        if program.identity.aliases.is_empty() {
            return Ok(());
        }
        if let Some(index) = self.grants.iter().position(|grant| grant.owner.task == task) {
            match admission::existing_grant(
                self.grants[index].owner.program.name() == program.name(),
                self.grants[index].delivered,
            ) {
                admission::ExistingGrant::Conflict => {
                    return Err("identity export task conflict");
                }
                admission::ExistingGrant::Delivered => return Ok(()),
                admission::ExistingGrant::Retry => {}
            }
            resource::port::ship(
                self.grants[index].endpoint,
                task,
                Access::STORE,
                env::Policy::NONE,
            )
            .map_err(|_| "identity export grant")?;
            self.grants[index].delivered = true;
            return Ok(());
        }
        if self.grants.len() >= EXPORT_CAP {
            return Err("identity export capacity");
        }
        let mut namespace_grants = Vec::new();
        namespace_grants
            .try_reserve_exact(program.operator.namespace.len())
            .map_err(|_| "identity export capacity")?;
        for (index, declaration) in program.operator.namespace.iter().copied().enumerate() {
            let grant = namespaces
                .grant(task, index)
                .ok_or("identity export namespace not prepared")?;
            namespace_grants.push(NamespaceGrant { declaration, grant });
        }
        self.grants
            .try_reserve(1)
            .map_err(|_| "identity export capacity")?;
        let endpoint = env::pie::unseal(env::UnsealArgs::hole(api::ENTRY))
            .map_err(|_| "identity export endpoint")?;
        self.grants.push(ExportGrant {
            owner: ExportOwner { task, program },
            endpoint,
            namespaces: namespace_grants,
            delivered: false,
        });
        let index = self.grants.len() - 1;
        resource::port::ship(endpoint, task, Access::STORE, env::Policy::NONE)
            .map_err(|_| "identity export grant")?;
        self.grants[index].delivered = true;
        Ok(())
    }

    /// Revokes the endpoint and drops only this task's export receipt.
    pub fn retire(&mut self, task: TaskId) -> Result<(), &'static str> {
        let Some(index) = self
            .grants
            .iter()
            .position(|grant| grant.owner.task == task)
        else {
            return Ok(());
        };
        env::pie::release(self.grants[index].endpoint, env::ReleaseMode::Revoke)
            .map_err(|_| "identity export revoke")?;
        self.grants.swap_remove(index);
        Ok(())
    }

    pub fn receive(
        &mut self,
        roster: &crate::system::control::identity::Roster,
        control: &crate::system::control::unit::Control,
    ) -> Result<(), crate::system::app::Fault> {
        use wire::Message;
        for grant in &self.grants {
            if !grant.delivered {
                continue;
            }
            let receiver = rpc::request::Receiver::<api::Call>::from_raw(
                grant.endpoint,
                api::Call::BACK,
                api::Call::back,
            );
            let mut bytes = api::Request::EMPTY;
            for _ in 0..16 {
                let incoming = match receiver.receive(&mut bytes, Wait::POLL) {
                    Ok(incoming) => incoming,
                    Err(rejected) if matches!(rejected.fail, rpc::Fail::Receive(_)) => break,
                    Err(rejected) => {
                        crate::debug::put(&alloc::format!("identity: export request rejected {:?}", rejected.fail));
                        continue;
                    },
                };
                let (request, exact) = incoming.request;
                let status = if !exact || !request.valid() {
                    crate::debug::put(&alloc::format!("identity: export bad request exact={exact} valid={}", request.valid()));
                    api::BAD
                } else {
                    self.associate(Admission {
                        context: AdmissionContext { roster, control },
                        grant,
                        caller: CallerRequest {
                            from: incoming.from,
                            request,
                        },
                    })
                    .err()
                    .map_or(api::OK, export_status)
                };
                let _ = incoming.reply.send(api::Response::rejected(status));
            }
        }
        Ok(())
    }

    fn associate(&self, admission: Admission<'_>) -> Result<(), ExportFailure> {
        let Admission {
            context,
            grant,
            caller,
        } = admission;
        let AdmissionContext { roster, control } = context;
        let CallerRequest { from, request } = caller;
        admission::authorize_alias(
            admission::CallerFacts {
                from,
                owner: grant.owner.task,
                live: control.live(from),
                named: control.task(grant.owner.program.name()) == Some(from),
            },
            admission::AliasClaim {
                aliases: grant.owner.program.identity.aliases,
                request: &request,
            },
            |task, subject| roster.allow_subject(task, subject).map_err(|why| {
                crate::debug::put(&alloc::format!("identity: export subject rejected: {why}"));
            }),
        )
        .map_err(|failure| {
            crate::debug::put(&alloc::format!("identity: export alias admission {failure:?}"));
            match failure {
            admission::Failure::Bad => ExportFailure::Bad,
            admission::Failure::Denied => ExportFailure::Denied,
        }})?;
        let binding_name = request.binding.as_str();
        let mut selected = None;
        for namespace in &grant.namespaces {
            let Some(declared) = namespace
                .declaration
                .bindings
                .iter()
                .find(|item| item.name == binding_name)
            else {
                continue;
            };
            if selected.is_some() {
                return Err(ExportFailure::Denied);
            }
            selected = Some((declared, namespace.grant));
        }
        let (declared, authority) = selected.ok_or_else(|| { crate::debug::put("identity: export binding undeclared"); ExportFailure::Denied })?;
        let root = self.root.as_ref().ok_or_else(|| { crate::debug::put("identity: export root absent"); ExportFailure::Denied })?;
        if Some(authority.entry.operator) != self.root_entry.map(|entry| entry.operator) {
            crate::debug::put("identity: export operator mismatch");
            return Err(ExportFailure::Denied);
        }
        let located = root
            .find(
                authority.entry,
                binding_name,
                system_api::operator::Permit::Bound,
                Wait::AtMost(500),
            )
            .map_err(|fail| {
                crate::debug::put(&alloc::format!("identity: export lookup {fail:?}"));
                ExportFailure::Denied
            })?;
        let actual = root.read(
            located.binding.entry,
            system_api::operator::Permit::Bound,
            Wait::AtMost(500),
        );
        if let Err(fail) = &actual { crate::debug::put(&alloc::format!("identity: export read {fail:?}")); }
        let actual_resource = actual.as_ref().ok().map(|actual| actual.resource);
        let result = actual.map_or(Err(ExportFailure::Denied), |actual| {
            self.verify_actual(ActualAdmission {
                trust: AdmissionTrust { roster, from },
                requested: AdmissionBinding { request, declared },
                lookup: ActualLookup {
                    located: located.binding,
                    actual,
                },
            })
        });
        admission::keep_lookup_resources(located.resource, actual_resource, |resource| {
            env::pie::release(resource, env::ReleaseMode::Keep).map_err(|_| ())
        })
        .map_err(|_| { crate::debug::put("identity: export lookup release failed"); ExportFailure::Denied })?;
        result
    }

    fn verify_actual(&self, admission: ActualAdmission<'_>) -> Result<(), ExportFailure> {
        let ActualAdmission {
            trust,
            requested,
            lookup,
        } = admission;
        let AdmissionTrust { roster, from } = trust;
        let AdmissionBinding { request, declared } = requested;
        let ActualLookup { located, actual } = lookup;
        let expected = actual_entry_ref(actual.binding.entry, actual.binding.revision);
        if expected != request.entry
            || actual.binding != located
            || actual.view.access() != declared.view
            || actual.permit != declared.permit
        {
            crate::debug::put("identity: export binding mismatch");
            return Err(ExportFailure::Denied);
        }
        let info = resource::raw::inspect(actual.resource).map_err(|why| {
            crate::debug::put(&alloc::format!("identity: export native inspect {why:?}"));
            ExportFailure::Denied
        })?;
        if !info.alive
            || info.owner != from
            || info.mark != declared.mark
            || info.permission != declared.view.bits()
        {
            crate::debug::put(&alloc::format!("identity: export source mismatch owner={} caller={} mark_match={} permission={:?} expected={:?} alive={}", info.owner.get(), from.get(), info.mark == declared.mark, info.permission, declared.view.bits(), info.alive));
            return Err(ExportFailure::Denied);
        }
        roster
            .associate(crate::system::control::identity::IdentityAssociation {
                object: request.object,
                name: request.alias,
                entry: expected,
            })
            .map_err(|why| {
                crate::debug::put(&alloc::format!("identity: export association {why}"));
                ExportFailure::Denied
            })
    }
}

pub fn bootstrap(
    layout: ::schedule::Res<crate::system::app::operator::Layout>,
    mut exports: ::schedule::ResMut<Exports>,
) -> Result<::schedule::Progress, crate::system::app::Fault> {
    let system = layout.as_ref().ok_or(crate::system::app::Fault::Dead)?;
    exports
        .bootstrap(system)
        .map_err(|_| crate::system::app::Fault::Dead)?;
    Ok(::schedule::Progress::Done)
}

pub fn receive(
    mut exports: ::schedule::ResMut<Exports>,
    roster: ::schedule::Res<crate::system::control::identity::Roster>,
    control: ::schedule::Res<crate::system::control::unit::Control>,
) -> Result<::schedule::Progress, crate::system::app::Fault> {
    exports.receive(&roster, &control)?;
    Ok(::schedule::Progress::Done)
}

#[derive(Clone, Copy)]
enum ExportFailure {
    Bad,
    Denied,
    Full,
}

fn export_status(failure: ExportFailure) -> u8 {
    match failure {
        ExportFailure::Bad => api::BAD,
        ExportFailure::Denied => api::DENIED,
        ExportFailure::Full => api::FULL,
    }
}

fn actual_entry_ref(entry: system_api::operator::Entry, revision: u64) -> OperatorEntryRef {
    OperatorEntryRef {
        operator: entry.operator,
        root: matches!(entry.target, system_api::operator::EntryTarget::Root),
        id: match entry.target {
            system_api::operator::EntryTarget::Root => 0,
            system_api::operator::EntryTarget::Node(id) => id.get() as u64,
        },
        revision,
    }
}
