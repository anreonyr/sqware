extern crate alloc;

mod collection;
mod lease;
mod pole;
pub(crate) mod retention;

use alloc::{boxed::Box, vec::Vec};
use env::Wait;
use env::{Mark, Permission, PieToken, TaskId};
use ipc::rpc;
use system_api::supply::{self, Delivery, Fail, Key, LeaseId, State};

pub use collection::{CollectionItem, CollectionResource, CollectionSpec};
#[allow(
    unused_imports,
    reason = "Parameter and route types for custom provider effects and resources."
)]
pub use lease::{Accord, DeliveryRoute, RootAssignment};
pub use lease::{
    Begin, Completion, DeliveryRequest, FailedRoots, LeaseKey, LeaseStart, Ledger, MAX_ROOTS,
    NativePieEffects, Origin, Parameters, PieEffects, PrivateRoot, Provision, ProvisionError,
    RootCheck, RootRequest,
};
pub use pole::{PoleEffects, PoleResource, PoleSpec};

pub struct LeaseRequest {
    pub id: LeaseId,
    pub origin: Origin,
    #[allow(
        dead_code,
        reason = "Provider admission context; built-in providers use caller and target."
    )]
    pub source: Key,
    pub resource: Key,
    pub target: TaskId,
    pub slot: Mark,
    pub permission: Permission,
    pub owner: TaskId,
}

impl LeaseRequest {
    pub fn derive(&self, shared: PieToken, index: usize) -> Result<PrivateRoot, Fail> {
        PrivateRoot::derive(
            shared,
            RootRequest {
                id: self.id,
                origin: self.origin,
                owner: self.owner,
                permission: self.permission,
            },
            index,
        )
    }
}

#[allow(
    dead_code,
    reason = "Asynchronous provider results; built-in providers complete acquisition synchronously."
)]
pub enum Recovery {
    Pending,
    Granted(Provision),
    Failed(ProvisionError),
    Released,
}

/// Resource definitions are registered by the provider's composition root.
/// Authorization belongs to each concrete provider and sees the native caller.
pub trait Resource {
    fn key(&self) -> Key;
    fn authorize(&self, request: &LeaseRequest) -> bool;
    fn acquire(&mut self, request: &LeaseRequest) -> Result<Provision, ProvisionError>;
    fn inspect(&mut self, _request: &LeaseRequest) -> Result<Recovery, Fail> {
        Ok(Recovery::Pending)
    }
    fn cancel(&mut self, request: &LeaseRequest) -> Result<(), Fail> {
        self.cleanup(request)
    }
    fn cleanup(&mut self, _request: &LeaseRequest) -> Result<(), Fail> {
        Ok(())
    }
    fn shutdown(&mut self) -> Result<(), Fail> {
        Ok(())
    }
}

/// Provider-local lease owner. One instance belongs to one actual native authority entry.
pub struct Source {
    config: SourceConfig,
    runtime: SourceRuntime,
}

struct SourceConfig {
    key: Key,
    authority: PieToken,
    owner: TaskId,
}

struct SourceRuntime {
    effects: Box<dyn PieEffects>,
    definitions: Vec<Box<dyn Resource>>,
    ledger: Ledger,
}

pub struct SourceSetup {
    pub key: Key,
    pub authority: PieToken,
    pub owner: TaskId,
    pub effects: Box<dyn PieEffects>,
}

#[must_use = "a rejected resource definition still owns its Pole and must be cleaned up"]
pub struct RegistrationError {
    failure: Fail,
    resource: Option<Box<dyn Resource>>,
}

impl core::fmt::Debug for RegistrationError {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter
            .debug_struct("RegistrationError")
            .field("failure", &self.failure())
            .finish()
    }
}

impl RegistrationError {
    pub fn failure(&self) -> Fail {
        self.failure
    }

    pub fn retry_cleanup(&mut self) -> Result<(), Fail> {
        let Some(resource) = self.resource.as_mut() else {
            return Ok(());
        };
        resource.shutdown()?;
        self.resource = None;
        Ok(())
    }
}

struct LeaseCall {
    actor: TaskId,
    requester: TaskId,
    id: LeaseId,
    parameters: Parameters,
}

struct SourceCall {
    actor: TaskId,
    requester: TaskId,
    id: LeaseId,
}

pub struct Incoming {
    pub actor: TaskId,
    pub requester: TaskId,
    pub request: supply::Request,
}

impl Source {
    pub fn new(key: Key, authority: PieToken) -> Result<Self, Fail> {
        let owner = env::unit::self_id();
        Self::new_with(SourceSetup {
            key,
            authority,
            owner,
            effects: Box::new(NativePieEffects),
        })
    }

    pub fn new_with(setup: SourceSetup) -> Result<Self, Fail> {
        let SourceSetup {
            key,
            authority,
            owner,
            mut effects,
        } = setup;
        if key.name().is_empty() || owner.get() == 0 {
            return Err(Fail::Bad);
        }
        let info = effects.inspect(authority)?;
        if !info.alive
            || info.kind != env::PieKind::Hole
            || !info.permission.contains(Permission::FETCH)
            || info.mark != supply::ASK
            || !authority_held_by(info, owner)
        {
            return Err(Fail::Denied);
        }
        Ok(Self {
            config: SourceConfig {
                key,
                authority,
                owner,
            },
            runtime: SourceRuntime {
                effects,
                definitions: Vec::new(),
                ledger: Ledger::new(),
            },
        })
    }

    pub fn key(&self) -> Key {
        self.config.key
    }
    /// Rejects exact duplicate names and hash collisions before the definition is reachable.
    pub fn register(&mut self, definition: Box<dyn Resource>) -> Result<(), RegistrationError> {
        let key = definition.key();
        if key.name().is_empty()
            || self
                .runtime
                .definitions
                .iter()
                .any(|old| old.key().mark() == key.mark() || old.key().name() == key.name())
        {
            return Err(RegistrationError {
                failure: Fail::Conflict,
                resource: Some(definition),
            });
        }
        if self.runtime.definitions.try_reserve(1).is_err() {
            return Err(RegistrationError {
                failure: Fail::Full,
                resource: Some(definition),
            });
        }
        self.runtime.definitions.push(definition);
        Ok(())
    }

    pub fn acquire(
        &mut self,
        id: LeaseId,
        parameters: Parameters,
    ) -> Result<(State, Option<Delivery>), Fail> {
        let actor = env::unit::self_id();
        self.acquire_from(LeaseCall {
            actor,
            requester: actor,
            id,
            parameters,
        })
    }

    fn acquire_from(&mut self, call: LeaseCall) -> Result<(State, Option<Delivery>), Fail> {
        let LeaseCall {
            actor,
            requester,
            id,
            parameters,
        } = call;
        let origin = self.origin_for(actor, requester)?;
        let start = LeaseStart {
            key: LeaseKey { id, origin },
            parameters,
        };
        if let Some(prior) = self.runtime.ledger.prior(start) {
            return match prior? {
                Begin::Existing(state, delivery, failure) => {
                    if let Some(failure) = failure {
                        return Err(failure);
                    }
                    match state {
                        State::Granted => Ok((state, delivery)),
                        State::Released => Ok((state, None)),
                        State::Closed => Err(Fail::Conflict),
                        _ => Err(Fail::Pending),
                    }
                }
                Begin::Started => Err(Fail::Bad),
            };
        }
        let source_key = self.config.key;
        let owner = self.config.owner;
        if parameters.source.mark() != source_key.mark()
            || parameters.target.get() == 0
            || parameters.permission.is_empty()
        {
            return Err(Fail::Denied);
        }
        let Some(definition) = self
            .runtime
            .definitions
            .iter()
            .find(|definition| definition.key().mark() == parameters.resource.mark())
        else {
            return Err(Fail::Denied);
        };
        let resource_key = definition.key();
        let request = LeaseRequest {
            id,
            origin,
            source: source_key,
            resource: resource_key,
            target: parameters.target,
            slot: parameters.slot,
            permission: parameters.permission,
            owner,
        };
        if !definition.authorize(&request) {
            return Err(Fail::Denied);
        }
        match self.runtime.ledger.begin(start)? {
            Begin::Existing(state, delivery, failure) => {
                if let Some(failure) = failure {
                    return Err(failure);
                }
                return match state {
                    State::Granted => Ok((state, delivery)),
                    State::Released => Ok((state, None)),
                    State::Closed => Err(Fail::Conflict),
                    _ => Err(Fail::Pending),
                };
            }
            Begin::Started => {}
        }
        let Some(definition) = self.definition_mut(parameters.resource) else {
            self.runtime
                .ledger
                .abort(LeaseKey { id, origin }, Fail::SourceLost)?;
            return Err(Fail::SourceLost);
        };
        let provision = match definition.acquire(&request) {
            Ok(provision) => provision,
            Err(error) => {
                let (failure, roots) = error.into_parts();
                if roots.is_empty() && matches!(failure, Fail::Pending | Fail::SourceLost) {
                    return Err(Fail::Pending);
                }
                if roots.is_empty() {
                    self.runtime
                        .ledger
                        .abort(LeaseKey { id, origin }, failure)?;
                } else {
                    self.runtime.ledger.abort_with_roots(
                        LeaseKey { id, origin },
                        FailedRoots { failure, roots },
                    )?;
                }
                self.cleanup(id, origin)?;
                return Err(failure);
            }
        };
        self.finish_provision(request, provision)
    }

    pub fn inspect(&mut self, id: LeaseId) -> Result<(State, Option<Delivery>), Fail> {
        let actor = env::unit::self_id();
        self.inspect_from(SourceCall {
            actor,
            requester: actor,
            id,
        })
    }

    fn inspect_from(&mut self, call: SourceCall) -> Result<(State, Option<Delivery>), Fail> {
        let SourceCall {
            actor,
            requester,
            id,
        } = call;
        let origin = self.origin_for(actor, requester)?;
        let (state, _delivery, failure) = self.runtime.ledger.inspect(id, origin)?;
        if let Some(failure) = failure {
            return Err(failure);
        }
        if state == State::Acquiring {
            let request = self.request_for(id, origin)?;
            let Some(definition) = self.definition_mut(request.resource) else {
                return Err(Fail::SourceLost);
            };
            match definition.inspect(&request)? {
                Recovery::Pending => return Ok((State::Acquiring, None)),
                Recovery::Failed(error) => {
                    let (failure, roots) = error.into_parts();
                    if roots.is_empty() {
                        self.runtime
                            .ledger
                            .abort(LeaseKey { id, origin }, failure)?;
                    } else {
                        self.runtime.ledger.abort_with_roots(
                            LeaseKey { id, origin },
                            FailedRoots { failure, roots },
                        )?;
                    }
                    self.cleanup(id, origin)?;
                    return Err(failure);
                }
                Recovery::Released => {
                    self.runtime
                        .ledger
                        .abort(LeaseKey { id, origin }, Fail::SourceLost)?;
                    self.cleanup(id, origin)?;
                    return Err(Fail::SourceLost);
                }
                Recovery::Granted(provision) => return self.finish_provision(request, provision),
            }
        }
        if state == State::Releasing {
            self.cleanup(id, origin)?;
        }
        let (state, delivery, failure) = self.runtime.ledger.inspect(id, origin)?;
        if let Some(failure) = failure {
            return Err(failure);
        }
        Ok((state, delivery))
    }

    #[allow(
        dead_code,
        reason = "Local counterpart of the provider RPC cancel operation."
    )]
    pub fn cancel(&mut self, id: LeaseId) -> Result<State, Fail> {
        let actor = env::unit::self_id();
        self.cancel_from(SourceCall {
            actor,
            requester: actor,
            id,
        })
    }

    fn cancel_from(&mut self, call: SourceCall) -> Result<State, Fail> {
        let SourceCall {
            actor,
            requester,
            id,
        } = call;
        let origin = self.origin_for(actor, requester)?;
        let state = self.runtime.ledger.cancel(id, origin)?;
        if state != State::Releasing {
            return Ok(state);
        }
        self.cleanup(id, origin)
    }

    pub fn release(&mut self, id: LeaseId) -> Result<State, Fail> {
        let actor = env::unit::self_id();
        self.release_from(SourceCall {
            actor,
            requester: actor,
            id,
        })
    }

    /// Revoke all leases before retiring provider-owned resource roots.
    pub fn shutdown(&mut self) -> Result<(), Fail> {
        let active = self.runtime.ledger.active();
        for (id, origin) in active {
            match self.release_from(SourceCall {
                actor: self.config.owner,
                requester: origin.requester,
                id,
            })? {
                State::Released | State::Closed => {}
                _ => return Err(Fail::Pending),
            }
        }
        for definition in &mut self.runtime.definitions {
            definition.shutdown()?;
        }
        Ok(())
    }

    fn release_from(&mut self, call: SourceCall) -> Result<State, Fail> {
        let SourceCall {
            actor,
            requester,
            id,
        } = call;
        let origin = self.origin_for(actor, requester)?;
        let state = self.runtime.ledger.release_begin(id, origin)?;
        if !matches!(state, State::Releasing) {
            return Ok(state);
        }
        self.cleanup(id, origin)
    }

    fn handle_from(&mut self, incoming: Incoming) -> supply::Response {
        let Incoming {
            actor,
            requester,
            request,
        } = incoming;
        let id = LeaseId(request.lease);
        let result = match request.op {
            supply::ACQUIRE => match Permission::from_bits(request.permission) {
                Some(permission) => self.acquire_from(LeaseCall {
                    actor,
                    requester,
                    id,
                    parameters: Parameters {
                        source: Key::from_mark(Mark::new(request.source)),
                        resource: Key::from_mark(Mark::new(request.resource)),
                        target: request.target,
                        slot: Mark::new(request.slot),
                        permission,
                    },
                }),
                None => Err(Fail::Bad),
            },
            supply::INSPECT => self.inspect_from(SourceCall {
                actor,
                requester,
                id,
            }),
            supply::CANCEL => self
                .cancel_from(SourceCall {
                    actor,
                    requester,
                    id,
                })
                .map(|state| (state, None)),
            supply::RELEASE => self
                .release_from(SourceCall {
                    actor,
                    requester,
                    id,
                })
                .map(|state| (state, None)),
            _ => Err(Fail::Bad),
        };
        match result {
            Ok((state, Some(delivery))) => response(wire::OK, state, Some(delivery)),
            Ok((state, None)) => response(wire::OK, state, None),
            Err(failure) => response(
                failure.code(),
                self.runtime
                    .ledger
                    .inspect(id, self.origin_unchecked(requester))
                    .map_or(State::Bad, |v| v.0),
                None,
            ),
        }
    }

    #[cfg(test)]
    pub fn test_handle_from(&mut self, incoming: Incoming) -> supply::Response {
        self.handle_from(incoming)
    }

    fn answer<F>(&mut self, incoming: Incoming, send: F) -> Result<(), Fail>
    where
        F: FnOnce(supply::Response) -> Result<(), Fail>,
    {
        let response = self.handle_from(incoming);
        send(response)
    }

    #[cfg(test)]
    pub fn test_answer<F>(&mut self, incoming: Incoming, send: F) -> Result<(), Fail>
    where
        F: FnOnce(supply::Response) -> Result<(), Fail>,
    {
        self.answer(incoming, send)
    }

    /// Receive and process one native request. A failed reply does not roll back an accepted lease.
    #[allow(
        dead_code,
        reason = "Provider RPC entry; current composition invokes sources directly."
    )]
    pub fn serve_once(&mut self, wait: Wait) -> Result<bool, Fail> {
        use wire::Message;
        let receiver = rpc::request::Receiver::<supply::Call>::from_raw(
            self.config.authority,
            supply::Call::BACK,
            supply::Call::back,
        );
        let mut buffer = <supply::Request as Message>::EMPTY;
        let incoming = match receiver.receive(buffer.as_mut(), wait) {
            Ok(incoming) => incoming,
            Err(rejected) if matches!(rejected.fail, rpc::Fail::Receive(_)) => return Ok(false),
            Err(_) => return Err(Fail::Bad),
        };
        let reply = incoming.reply;
        let request = Incoming {
            actor: env::unit::self_id(),
            requester: incoming.from,
            request: incoming.request,
        };
        let _ = self.answer(request, |said| {
            reply.send(said).map_err(|_| Fail::SourceLost)
        });
        Ok(true)
    }

    fn finish_provision(
        &mut self,
        request: LeaseRequest,
        provision: Provision,
    ) -> Result<(State, Option<Delivery>), Fail> {
        if provision.roots.is_empty()
            || provision.roots.len() > MAX_ROOTS
            || provision.delivery_root >= provision.roots.len()
            || provision.description_len as usize > supply::MAX_DESCRIPTION
            || provision.routes.iter().enumerate().any(|(at, route)| {
                route.root >= provision.roots.len()
                    || route.root == provision.delivery_root
                    || route.slot == Mark::NONE
                    || route.permission.is_empty()
                    || provision.routes[..at]
                        .iter()
                        .any(|old| old.root == route.root || old.slot == route.slot)
            })
        {
            self.runtime.ledger.abort_with_roots(
                LeaseKey {
                    id: request.id,
                    origin: request.origin,
                },
                FailedRoots {
                    failure: Fail::Bad,
                    roots: provision.roots,
                },
            )?;
            let _ = self.cleanup(request.id, request.origin);
            return Err(Fail::Bad);
        }
        if provision.roots.iter().enumerate().any(|(index, root)| {
            !root.valid_for(RootCheck {
                key: LeaseKey {
                    id: request.id,
                    origin: request.origin,
                },
                index,
                owner: self.config.owner,
            })
        }) {
            self.runtime.ledger.abort_with_roots(
                LeaseKey {
                    id: request.id,
                    origin: request.origin,
                },
                FailedRoots {
                    failure: Fail::SourceLost,
                    roots: provision.roots,
                },
            )?;
            let _ = self.cleanup(request.id, request.origin);
            return Err(Fail::SourceLost);
        }
        if self.runtime.ledger.inspect(request.id, request.origin)?.0 != State::Acquiring {
            self.runtime.ledger.complete(
                LeaseKey {
                    id: request.id,
                    origin: request.origin,
                },
                Completion {
                    delivery: Delivery::EMPTY,
                    roots: provision.roots,
                },
            )?;
            let state = self.cleanup(request.id, request.origin)?;
            return Ok((state, None));
        }
        for route in &provision.routes {
            if let Err(failure) = provision.roots[route.root].deliver_with(
                DeliveryRequest {
                    target: request.target,
                    slot: route.slot,
                    permission: route.permission,
                    description: &[],
                },
                self.runtime.effects.as_mut(),
            ) {
                self.runtime.ledger.abort_with_roots(
                    LeaseKey {
                        id: request.id,
                        origin: request.origin,
                    },
                    FailedRoots {
                        failure,
                        roots: provision.roots,
                    },
                )?;
                let _ = self.cleanup(request.id, request.origin);
                return Err(failure);
            }
        }
        let delivery = match provision.roots[provision.delivery_root].deliver_with(
            DeliveryRequest {
                target: request.target,
                slot: request.slot,
                permission: request.permission,
                description: &provision.description[..provision.description_len as usize],
            },
            self.runtime.effects.as_mut(),
        ) {
            Ok(delivery) => delivery,
            Err(failure) => {
                self.runtime.ledger.abort_with_roots(
                    LeaseKey {
                        id: request.id,
                        origin: request.origin,
                    },
                    FailedRoots {
                        failure,
                        roots: provision.roots,
                    },
                )?;
                let _ = self.cleanup(request.id, request.origin);
                return Err(failure);
            }
        };
        let complete = self.runtime.ledger.complete(
            LeaseKey {
                id: request.id,
                origin: request.origin,
            },
            Completion {
                delivery,
                roots: provision.roots,
            },
        )?;
        if !complete {
            let state = self.cleanup(request.id, request.origin)?;
            return Ok((state, None));
        }
        Ok((State::Granted, Some(delivery)))
    }

    fn cleanup(&mut self, id: LeaseId, origin: Origin) -> Result<State, Fail> {
        let request = self.request_for(id, origin)?;
        let cancelled = self.runtime.ledger.is_cancelled(id, origin)?;
        let definition = self
            .definition_mut(request.resource)
            .ok_or(Fail::SourceLost)?;
        if cancelled {
            definition.cancel(&request)?;
        } else {
            definition.cleanup(&request)?;
        }
        {
            let roots = self.runtime.ledger.roots_mut(id, origin)?;
            revoke_roots(roots, self.runtime.effects.as_mut())?;
        }
        self.runtime.ledger.finish_release(id, origin)
    }

    fn request_for(&self, id: LeaseId, origin: Origin) -> Result<LeaseRequest, Fail> {
        let (state, _, _) = self.runtime.ledger.inspect(id, origin)?;
        let _ = state;
        // Logical parameters are retained by the ledger and are exposed without transport fields.
        let parameters = self.runtime.ledger.parameters(id, origin)?;
        let resource = self
            .runtime
            .definitions
            .iter()
            .find(|definition| definition.key().mark() == parameters.resource.mark())
            .map(|definition| definition.key())
            .ok_or(Fail::SourceLost)?;
        Ok(LeaseRequest {
            id,
            origin,
            source: self.config.key,
            resource,
            target: parameters.target,
            slot: parameters.slot,
            permission: parameters.permission,
            owner: self.config.owner,
        })
    }

    fn origin_for(&mut self, actor: TaskId, requester: TaskId) -> Result<Origin, Fail> {
        let info = self.runtime.effects.inspect(self.config.authority)?;
        if !info.alive
            || info.kind != env::PieKind::Hole
            || !info.permission.contains(Permission::FETCH)
            || actor != self.config.owner
            || !authority_held_by(info, actor)
            || info.mark != supply::ASK
        {
            return Err(Fail::SourceLost);
        }
        Origin::bind(lease::OriginBinding {
            provider: self.config.owner,
            actor,
            requester,
            authority: self.config.authority.get(),
        })
    }

    fn origin_unchecked(&self, requester: TaskId) -> Origin {
        Origin {
            requester,
            authority: self.config.authority.get(),
        }
    }

    fn definition_mut(&mut self, key: Key) -> Option<&mut Box<dyn Resource>> {
        self.runtime
            .definitions
            .iter_mut()
            .find(|definition| definition.key().mark() == key.mark())
    }
}

fn authority_held_by(info: env::PieInfo, actor: TaskId) -> bool {
    info.owner == actor || info.vestor == actor
}

fn revoke_roots<E: PieEffects + ?Sized>(
    roots: &mut Vec<PrivateRoot>,
    effects: &mut E,
) -> Result<(), Fail> {
    let mut first = None;
    for root in roots.iter_mut() {
        if let Err(failure) = root.revoke_with(effects) {
            first.get_or_insert(failure);
        }
    }
    roots.retain(|root| root.token() != PieToken::NONE);
    first.map_or(Ok(()), Err)
}

fn response(status: u8, state: State, delivery: Option<Delivery>) -> supply::Response {
    let mut response = supply::Response {
        status,
        state: state.code(),
        seed: env::PieToken::NONE,
        len: 0,
        description: [0; supply::MAX_DESCRIPTION],
    };
    if let Some(delivery) = delivery {
        response.seed = delivery.seed;
        response.len = delivery.len;
        response.description = delivery.description;
    }
    response
}
