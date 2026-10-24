use alloc::vec::Vec;
use env::{GrantRange, Mark, Permission, PieKind, PieToken, TaskId};
use system_api::supply::{self, Delivery, Fail, Key, LeaseId, State};

use super::derivation::Derivation;

pub const MAX_CALLERS: usize = 64;
/// A provider may retain one delivery root and sixty-four separately revoked roots.
pub const MAX_ROOTS: usize = 65;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Origin {
    pub requester: TaskId,
    /// The provider's actual local authority endpoint token.
    pub authority: usize,
}

#[derive(Clone, Copy)]
pub struct OriginBinding {
    pub provider: TaskId,
    pub actor: TaskId,
    pub requester: TaskId,
    pub authority: usize,
}

impl Origin {
    pub fn bind(binding: OriginBinding) -> Result<Self, Fail> {
        if binding.provider.get() == 0
            || binding.actor != binding.provider
            || binding.requester.get() == 0
            || binding.authority == 0
        {
            return Err(Fail::Denied);
        }
        Ok(Self {
            requester: binding.requester,
            authority: binding.authority,
        })
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Parameters {
    pub source: Key,
    pub resource: Key,
    pub target: TaskId,
    pub slot: Mark,
    pub permission: Permission,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LeaseKey {
    pub id: LeaseId,
    pub origin: Origin,
}

#[derive(Clone, Copy)]
pub struct LeaseStart {
    pub key: LeaseKey,
    pub parameters: Parameters,
}

pub struct Completion {
    pub delivery: Delivery,
    pub roots: Vec<PrivateRoot>,
}

pub struct FailedRoots {
    pub failure: Fail,
    pub roots: Vec<PrivateRoot>,
}

pub struct RootAssignment {
    pub shared: PieToken,
    pub index: usize,
    /// 本地派生的**有效窗口**（相对来源起点的绝对页偏移与页数）。
    ///
    /// [`GrantRange::Inherit`] 让子分支**继承**父分支那一段（父不带窗口时才是整段）。
    /// 只有手里就是那枚来源的真实管理者才拿得出这对坐标；合法性（落在父窗口里、非空）归内核。
    pub range: GrantRange,
}

/// 跨任务交付一段子分支；范围总是继承父分支，不能在这里另开窗口。
pub struct Accord {
    pub token: PieToken,
    pub target: TaskId,
    pub permission: Permission,
    pub mark: Mark,
}

pub struct DeliveryRequest<'a> {
    pub target: TaskId,
    pub slot: Mark,
    pub permission: Permission,
    pub description: &'a [u8],
}

pub struct RootCheck {
    pub key: LeaseKey,
    pub index: usize,
    pub owner: TaskId,
}

#[derive(Clone, Copy)]
struct WatermarkState {
    state: State,
    failure: Option<Fail>,
}

#[derive(Clone, Copy)]
pub struct RootRequest {
    pub id: LeaseId,
    pub origin: Origin,
    pub owner: TaskId,
    pub permission: Permission,
}

pub struct Provision {
    pub roots: Vec<PrivateRoot>,
    pub delivery_root: usize,
    pub routes: Vec<DeliveryRoute>,
    pub description: [u8; supply::MAX_DESCRIPTION],
    pub description_len: u16,
}

#[derive(Clone, Copy)]
pub struct DeliveryRoute {
    pub root: usize,
    pub slot: Mark,
    pub permission: Permission,
}

impl Provision {
    pub fn new(
        roots: Vec<PrivateRoot>,
        delivery_root: usize,
        description: &[u8],
    ) -> Result<Self, ProvisionError> {
        if roots.is_empty()
            || roots.len() > MAX_ROOTS
            || delivery_root >= roots.len()
            || description.len() > supply::MAX_DESCRIPTION
        {
            return Err(ProvisionError::with_roots(Fail::Bad, roots));
        }
        let mut bytes = [0; supply::MAX_DESCRIPTION];
        bytes[..description.len()].copy_from_slice(description);
        Ok(Self {
            roots,
            delivery_root,
            routes: Vec::new(),
            description: bytes,
            description_len: description.len() as u16,
        })
    }

    pub fn route(&mut self, route: DeliveryRoute) -> Result<(), Fail> {
        if route.slot == Mark::NONE || route.permission.is_empty() {
            return Err(Fail::Bad);
        }
        self.routes.try_reserve(1).map_err(|_| Fail::Full)?;
        self.routes.push(route);
        Ok(())
    }
}

pub struct ProvisionError {
    pub failure: Fail,
    roots: Vec<PrivateRoot>,
}

impl ProvisionError {
    pub fn new(failure: Fail) -> Self {
        Self {
            failure,
            roots: Vec::new(),
        }
    }
    /// Preserve every private root created before a provider failed partway through provisioning.
    pub fn with_roots(failure: Fail, roots: Vec<PrivateRoot>) -> Self {
        Self { failure, roots }
    }
    pub fn into_parts(self) -> (Fail, Vec<PrivateRoot>) {
        (self.failure, self.roots)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Begin {
    Started,
    Existing(State, Option<Delivery>, Option<Fail>),
}

#[derive(Clone, Copy)]
struct Watermark {
    origin: Origin,
    last: u64,
    last_parameters: Option<Parameters>,
    last_state: State,
    last_failure: Option<Fail>,
}

struct Record {
    request: LeaseRequestData,
    phase: LeasePhase,
    roots: Vec<PrivateRoot>,
}

struct LeaseRequestData {
    key: LeaseKey,
    parameters: Option<Parameters>,
}

#[derive(Clone, Copy)]
enum LeasePhase {
    Acquiring,
    Granted(Delivery),
    Releasing {
        cancelled: bool,
        failure: Option<Fail>,
        delivery: Option<Delivery>,
    },
    Released,
    Closed(Option<Fail>),
}

impl LeasePhase {
    fn state(&self) -> State {
        match self {
            Self::Acquiring => State::Acquiring,
            Self::Granted(_) => State::Granted,
            Self::Releasing { .. } => State::Releasing,
            Self::Released => State::Released,
            Self::Closed(_) => State::Closed,
        }
    }

    fn delivery(&self) -> Option<Delivery> {
        match self {
            Self::Granted(delivery) => Some(*delivery),
            Self::Releasing { delivery, .. } => *delivery,
            _ => None,
        }
    }

    fn failure(&self) -> Option<Fail> {
        match self {
            Self::Releasing { failure, .. } | Self::Closed(failure) => *failure,
            _ => None,
        }
    }

    fn cancelled(&self) -> bool {
        matches!(
            self,
            Self::Releasing {
                cancelled: true,
                ..
            }
        )
    }
}

pub struct Ledger {
    records: Vec<Record>,
    watermarks: Vec<Watermark>,
}

impl Ledger {
    pub fn new() -> Self {
        Self {
            records: Vec::new(),
            watermarks: Vec::new(),
        }
    }

    pub fn begin(&mut self, start: LeaseStart) -> Result<Begin, Fail> {
        let LeaseStart {
            key: LeaseKey { id, origin },
            parameters,
        } = start;
        if let Some(prior) = self.prior(start) {
            return prior;
        }
        self.reserve_record()?;
        let watermark = self
            .watermarks
            .iter()
            .position(|item| item.origin == origin);
        match watermark {
            Some(at) if id.0 <= self.watermarks[at].last => return Err(Fail::Conflict),
            Some(at) => {
                self.watermarks[at] = Watermark {
                    origin,
                    last: id.0,
                    last_parameters: Some(parameters),
                    last_state: State::Acquiring,
                    last_failure: None,
                }
            }
            None => {
                if self.watermarks.len() == MAX_CALLERS {
                    return Err(Fail::Full);
                }
                self.watermarks.try_reserve(1).map_err(|_| Fail::Full)?;
                self.watermarks.push(Watermark {
                    origin,
                    last: id.0,
                    last_parameters: Some(parameters),
                    last_state: State::Acquiring,
                    last_failure: None,
                });
            }
        }
        self.records.push(Record {
            request: LeaseRequestData {
                key: LeaseKey { id, origin },
                parameters: Some(parameters),
            },
            phase: LeasePhase::Acquiring,
            roots: Vec::new(),
        });
        Ok(Begin::Started)
    }

    /// Read the request history without reserving a caller watermark or lease record.
    pub fn prior(&self, start: LeaseStart) -> Option<Result<Begin, Fail>> {
        let LeaseStart {
            key: LeaseKey { id, origin },
            parameters,
        } = start;
        if let Some(record) = self
            .records
            .iter()
            .find(|record| record.request.key.id == id && record.request.key.origin == origin)
        {
            return Some(if record.request.parameters == Some(parameters) {
                Ok(Begin::Existing(
                    record.phase.state(),
                    record.phase.delivery(),
                    record.phase.failure(),
                ))
            } else {
                Err(Fail::Conflict)
            });
        }
        if let Some(watermark) = self
            .watermarks
            .iter()
            .find(|item| item.origin == origin && item.last == id.0)
        {
            return Some(if watermark.last_parameters == Some(parameters) {
                Ok(Begin::Existing(
                    watermark.last_state,
                    None,
                    watermark.last_failure,
                ))
            } else {
                Err(Fail::Conflict)
            });
        }
        self.watermarks
            .iter()
            .any(|item| item.origin == origin && id.0 <= item.last)
            .then_some(Err(Fail::Conflict))
    }

    pub fn cancel(&mut self, id: LeaseId, origin: Origin) -> Result<State, Fail> {
        if let Some(at) = self
            .records
            .iter()
            .position(|record| record.request.key.id == id && record.request.key.origin == origin)
        {
            let record = &mut self.records[at];
            return match record.phase {
                LeasePhase::Acquiring => {
                    record.phase = LeasePhase::Releasing {
                        cancelled: true,
                        failure: None,
                        delivery: None,
                    };
                    let failure = None;
                    self.watermark_state(
                        LeaseKey { id, origin },
                        WatermarkState {
                            state: State::Releasing,
                            failure,
                        },
                    );
                    Ok(State::Releasing)
                }
                _ => Ok(record.phase.state()),
            };
        }
        self.reserve_record()?;
        let watermark = self
            .watermarks
            .iter()
            .position(|item| item.origin == origin);
        match watermark {
            Some(at) if id.0 <= self.watermarks[at].last => return Err(Fail::Conflict),
            Some(at) => {
                self.watermarks[at] = Watermark {
                    origin,
                    last: id.0,
                    last_parameters: None,
                    last_state: State::Closed,
                    last_failure: None,
                }
            }
            None => {
                if self.watermarks.len() == MAX_CALLERS {
                    return Err(Fail::Full);
                }
                self.watermarks.try_reserve(1).map_err(|_| Fail::Full)?;
                self.watermarks.push(Watermark {
                    origin,
                    last: id.0,
                    last_parameters: None,
                    last_state: State::Closed,
                    last_failure: None,
                });
            }
        }
        self.records.push(Record {
            request: LeaseRequestData {
                key: LeaseKey { id, origin },
                parameters: None,
            },
            phase: LeasePhase::Closed(None),
            roots: Vec::new(),
        });
        Ok(State::Closed)
    }

    pub fn abort(&mut self, key: LeaseKey, failure: Fail) -> Result<(), Fail> {
        let state = self.find_record(key)?.phase.state();
        match state {
            State::Acquiring => {
                let record = self.find_mut(key)?;
                record.phase = LeasePhase::Releasing {
                    cancelled: true,
                    failure: Some(failure),
                    delivery: None,
                };
                self.watermark_state(
                    key,
                    WatermarkState {
                        state: State::Releasing,
                        failure: Some(failure),
                    },
                );
                Ok(())
            }
            State::Releasing => {
                let record = self.find_mut(key)?;
                let LeasePhase::Releasing {
                    cancelled,
                    failure: prior,
                    delivery,
                } = record.phase
                else {
                    unreachable!()
                };
                let recorded = prior.or(Some(failure));
                record.phase = LeasePhase::Releasing {
                    cancelled,
                    failure: recorded,
                    delivery,
                };
                self.watermark_state(
                    key,
                    WatermarkState {
                        state: State::Releasing,
                        failure: recorded,
                    },
                );
                Ok(())
            }
            State::Closed => Ok(()),
            _ => Err(Fail::Conflict),
        }
    }

    /// Finish an accepted provider operation. A canceled/releasing request cannot become granted.
    pub fn complete(&mut self, key: LeaseKey, completed: Completion) -> Result<bool, Fail> {
        let Completion { delivery, roots } = completed;
        let record = self.find_mut(key)?;
        if roots.is_empty() || roots.len() > MAX_ROOTS {
            return Err(Fail::Bad);
        }
        let complete = match record.phase.state() {
            State::Acquiring => {
                record.roots = roots;
                record.phase = LeasePhase::Granted(delivery);
                Ok(true)
            }
            State::Closed | State::Releasing | State::Released => {
                record.roots = roots;
                if let LeasePhase::Releasing {
                    cancelled, failure, ..
                } = record.phase
                {
                    record.phase = LeasePhase::Releasing {
                        cancelled,
                        failure,
                        delivery: None,
                    };
                }
                Ok(false)
            }
            State::Granted => Ok(true),
            State::Bad | State::Unseen => Err(Fail::Conflict),
        }?;
        let state = self.find_record(key)?.phase.state();
        self.watermark_state(
            key,
            WatermarkState {
                state,
                failure: None,
            },
        );
        Ok(complete)
    }

    pub fn abort_with_roots(&mut self, key: LeaseKey, failed: FailedRoots) -> Result<(), Fail> {
        let FailedRoots { failure, roots } = failed;
        let record = self.find_mut(key)?;
        record.roots = roots;
        record.phase = LeasePhase::Releasing {
            cancelled: true,
            failure: Some(failure),
            delivery: None,
        };
        self.watermark_state(
            key,
            WatermarkState {
                state: State::Releasing,
                failure: Some(failure),
            },
        );
        Ok(())
    }

    pub fn release_begin(&mut self, id: LeaseId, origin: Origin) -> Result<State, Fail> {
        if let Some(at) = self
            .records
            .iter()
            .position(|record| record.request.key.id == id && record.request.key.origin == origin)
        {
            let state = self.records[at].phase.state();
            return match state {
                State::Acquiring => {
                    self.records[at].phase = LeasePhase::Releasing {
                        cancelled: true,
                        failure: None,
                        delivery: None,
                    };
                    self.watermark_state(
                        LeaseKey { id, origin },
                        WatermarkState {
                            state: State::Releasing,
                            failure: None,
                        },
                    );
                    Ok(State::Releasing)
                }
                State::Granted => {
                    let delivery = self.records[at].phase.delivery();
                    self.records[at].phase = LeasePhase::Releasing {
                        cancelled: false,
                        failure: None,
                        delivery,
                    };
                    self.watermark_state(
                        LeaseKey { id, origin },
                        WatermarkState {
                            state: State::Releasing,
                            failure: None,
                        },
                    );
                    Ok(State::Releasing)
                }
                state => Ok(state),
            };
        }
        self.cancel(id, origin)
    }

    pub fn roots_mut(
        &mut self,
        id: LeaseId,
        origin: Origin,
    ) -> Result<&mut Vec<PrivateRoot>, Fail> {
        Ok(&mut self.find_mut(LeaseKey { id, origin })?.roots)
    }

    pub fn parameters(&self, id: LeaseId, origin: Origin) -> Result<Parameters, Fail> {
        self.records
            .iter()
            .find(|record| record.request.key.id == id && record.request.key.origin == origin)
            .and_then(|record| record.request.parameters)
            .ok_or(Fail::Unknown)
    }

    pub fn is_cancelled(&self, id: LeaseId, origin: Origin) -> Result<bool, Fail> {
        Ok(self.find_record(LeaseKey { id, origin })?.phase.cancelled())
    }

    pub fn active(&self) -> Vec<(LeaseId, Origin)> {
        self.records
            .iter()
            .filter(|record| {
                matches!(
                    record.phase.state(),
                    State::Acquiring | State::Granted | State::Releasing
                )
            })
            .map(|record| (record.request.key.id, record.request.key.origin))
            .collect()
    }

    fn find_record(&self, key: LeaseKey) -> Result<&Record, Fail> {
        self.records
            .iter()
            .find(|record| record.request.key == key)
            .ok_or(Fail::Unknown)
    }

    pub fn finish_release(&mut self, id: LeaseId, origin: Origin) -> Result<State, Fail> {
        let at = self
            .records
            .iter()
            .position(|record| record.request.key.id == id && record.request.key.origin == origin)
            .ok_or(Fail::Unknown)?;
        if !self.records[at].roots.is_empty() {
            return Err(Fail::Pending);
        }
        if let LeasePhase::Releasing {
            cancelled, failure, ..
        } = self.records[at].phase
        {
            self.records[at].phase = if cancelled {
                LeasePhase::Closed(failure)
            } else {
                LeasePhase::Released
            };
        }
        let state = self.records[at].phase.state();
        let failure = self.records[at].phase.failure();
        self.watermark_state(LeaseKey { id, origin }, WatermarkState { state, failure });
        if matches!(state, State::Released | State::Closed) {
            self.records.remove(at);
        }
        Ok(state)
    }

    pub fn inspect(
        &self,
        id: LeaseId,
        origin: Origin,
    ) -> Result<(State, Option<Delivery>, Option<Fail>), Fail> {
        let Some(record) = self
            .records
            .iter()
            .find(|record| record.request.key.id == id && record.request.key.origin == origin)
        else {
            if let Some(watermark) = self
                .watermarks
                .iter()
                .find(|watermark| watermark.origin == origin && watermark.last == id.0)
            {
                return Ok((watermark.last_state, None, watermark.last_failure));
            }
            if self
                .watermarks
                .iter()
                .any(|watermark| watermark.origin == origin && id.0 <= watermark.last)
            {
                return Ok((State::Closed, None, None));
            }
            return Err(Fail::Unknown);
        };
        Ok((
            record.phase.state(),
            record.phase.delivery(),
            record.phase.failure(),
        ))
    }

    fn find_mut(&mut self, key: LeaseKey) -> Result<&mut Record, Fail> {
        let record = self
            .records
            .iter_mut()
            .find(|record| record.request.key == key)
            .ok_or(Fail::Unknown)?;
        Ok(record)
    }

    fn reserve_record(&mut self) -> Result<(), Fail> {
        while self.records.len() >= supply::MAX_LEASES {
            let Some(at) = self.records.iter().position(|record| {
                matches!(record.phase.state(), State::Released | State::Closed)
                    && record.roots.is_empty()
            }) else {
                return Err(Fail::Full);
            };
            self.records.remove(at);
        }
        self.records.try_reserve(1).map_err(|_| Fail::Full)
    }

    fn watermark_state(&mut self, key: LeaseKey, update: WatermarkState) {
        if let Some(watermark) = self
            .watermarks
            .iter_mut()
            .find(|watermark| watermark.origin == key.origin && watermark.last == key.id.0)
        {
            watermark.last_state = update.state;
            watermark.last_failure = update.failure;
        }
    }
}

impl Default for Ledger {
    fn default() -> Self {
        Self::new()
    }
}

/// A private derived root owned by the source. Its Revoke closes every transferred descendant.
pub struct PrivateRoot {
    token: PieToken,
    source: TaskId,
    resource_owner: TaskId,
    lease_mark: Mark,
    kind: PieKind,
    permission: Permission,
    lease: LeaseId,
    index: usize,
    valid: bool,
}

pub trait PieEffects {
    fn inspect(&mut self, token: PieToken) -> Result<env::PieInfo, Fail>;
    fn derive(&mut self, request: Derivation) -> Result<PieToken, Fail>;
    fn accord(&mut self, request: Accord) -> Result<PieToken, Fail>;
    fn revoke(&mut self, token: PieToken) -> Result<(), Fail>;
}

pub struct NativePieEffects;

impl PieEffects for NativePieEffects {
    fn inspect(&mut self, token: PieToken) -> Result<env::PieInfo, Fail> {
        resource::raw::inspect(token).map_err(|_| Fail::SourceLost)
    }

    fn derive(&mut self, request: Derivation) -> Result<PieToken, Fail> {
        env::pie::derive(
            request.source,
            request.permission,
            request.mark,
            request.range,
        )
        .map_err(|_| Fail::Denied)
    }

    fn accord(&mut self, request: Accord) -> Result<PieToken, Fail> {
        env::pie::accord(
            request.token,
            request.target,
            request.permission,
            request.mark,
        )
        .map_err(|_| Fail::Denied)
    }

    fn revoke(&mut self, token: PieToken) -> Result<(), Fail> {
        resource::raw::release(token).map_err(|_| Fail::SourceLost)
    }
}

impl PrivateRoot {
    pub fn derive(assignment: RootAssignment, request: RootRequest) -> Result<Self, Fail> {
        Self::derive_with(assignment, request, &mut NativePieEffects)
    }

    pub fn derive_with<E: PieEffects + ?Sized>(
        assignment: RootAssignment,
        request: RootRequest,
        effects: &mut E,
    ) -> Result<Self, Fail> {
        let RootAssignment {
            shared,
            index,
            range,
        } = assignment;
        let source = effects.inspect(shared).map_err(|_| Fail::Denied)?;
        if !source.alive
            || request.owner.get() == 0
            || request.permission.is_empty()
            || request.permission.contains(Permission::ONLY)
                != source.permission.contains(Permission::ONLY)
        {
            return Err(Fail::Denied);
        }
        let mut child = request.permission | Permission::VEST;
        if source.permission.contains(Permission::ONLY) {
            child |= Permission::ONLY;
        }
        if !source.permission.contains(child) {
            return Err(Fail::Denied);
        }
        let lease_mark = lease_mark(request.id, request.origin, index);
        let token = effects.derive(Derivation {
            source: shared,
            permission: child,
            mark: lease_mark,
            range,
        })?;
        let valid = effects.inspect(token).is_ok_and(|derived| {
            derived.alive
                && derived.owner == source.owner
                && derived.vestor == request.owner
                && derived.mark == lease_mark
                && derived.kind == source.kind
                && derived.permission == child
        });
        Ok(Self {
            token,
            source: request.owner,
            resource_owner: source.owner,
            lease_mark,
            kind: source.kind,
            permission: child,
            lease: request.id,
            index,
            valid,
        })
    }

    pub fn deliver_with<E: PieEffects + ?Sized>(
        &self,
        request: DeliveryRequest<'_>,
        effects: &mut E,
    ) -> Result<Delivery, Fail> {
        let DeliveryRequest {
            target,
            slot,
            permission,
            description,
        } = request;
        if target.get() == 0
            || permission.is_empty()
            || permission.contains(Permission::ONLY) != self.permission.contains(Permission::ONLY)
            || !self.permission.contains(permission)
        {
            return Err(Fail::Denied);
        }
        let seed = effects.accord(Accord {
            token: self.token,
            target,
            permission,
            mark: slot,
        })?;
        Delivery::with_description(seed, description).ok_or(Fail::Bad)
    }

    pub fn token(&self) -> PieToken {
        self.token
    }
    pub fn valid_for(&self, check: RootCheck) -> bool {
        self.valid
            && self.lease == check.key.id
            && self.index == check.index
            && self.source == check.owner
            && self.lease_mark == lease_mark(check.key.id, check.key.origin, check.index)
    }

    pub fn revoke_with<E: PieEffects + ?Sized>(&mut self, effects: &mut E) -> Result<(), Fail> {
        if self.token == PieToken::NONE {
            return Ok(());
        }
        let info = effects.inspect(self.token)?;
        if !info.alive {
            self.token = PieToken::NONE;
            return Ok(());
        }
        if info.owner != self.resource_owner
            || info.vestor != self.source
            || info.mark != self.lease_mark
            || info.kind != self.kind
        {
            return Err(Fail::Denied);
        }
        if !info.permission.contains(self.permission) {
            return Err(Fail::Denied);
        }
        effects.revoke(self.token)?;
        self.token = PieToken::NONE;
        Ok(())
    }
}

fn lease_mark(id: LeaseId, origin: Origin, index: usize) -> Mark {
    Mark::new(
        id.0.rotate_left(17)
            ^ (origin.requester.get() as u64).rotate_left(31)
            ^ (origin.authority as u64).rotate_left(7)
            ^ (index as u64).wrapping_mul(0x9e37_79b9_7f4a_7c15)
            ^ 0x5355_5050_4c59_0001,
    )
}
