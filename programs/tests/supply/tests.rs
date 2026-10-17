use super::lease::{
    Accord, Begin, DeliveryRequest, FailedRoots, LeaseKey, LeaseStart, Ledger, Origin,
    OriginBinding, Parameters, PieEffects, PrivateRoot, RootAssignment, RootRequest,
};
use env::{Mark, Permission, PieInfo, PieKind, PieToken, TaskId};
use system_api::supply::{Fail, Key, LeaseId, State};

const SOURCE: Key = Key::of("vendor/source");
const VALID: system_api::supply::Binding = system_api::supply::Binding {
    source: SOURCE,
    resource: Key::of("vendor/resource"),
    slot: Mark::of("slot"),
    permission: Permission::FETCH,
};
const DENIED: system_api::supply::Binding = system_api::supply::Binding {
    permission: Permission::empty(),
    ..VALID
};
static VALID_BINDINGS: [system_api::supply::Binding; 1] = [VALID];
static DENIED_BINDINGS: [system_api::supply::Binding; 1] = [DENIED];
static DUPLICATE_SOURCES: [Key; 2] = [SOURCE, SOURCE];
static ONE_SOURCE: [Key; 1] = [SOURCE];
static NO_SOURCES: [Key; 0] = [];

fn origin(requester: usize, authority: usize) -> Origin {
    Origin {
        requester: TaskId::new(requester),
        authority,
    }
}

fn parameters(permission: Permission) -> Parameters {
    Parameters {
        source: Key::of("vendor/source"),
        resource: Key::of("vendor/resource"),
        target: TaskId::new(22),
        slot: Mark::of("target/resource"),
        permission,
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum PoleStep {
    Unseal,
    Open,
    Write,
    Shut,
    Inspect,
    Release,
}

struct PoleMock {
    root: Option<PieInfo>,
    mapped: bool,
    failures: Vec<(PoleStep, Fail)>,
}

impl PoleMock {
    fn new() -> Self {
        Self {
            root: None,
            mapped: false,
            failures: Vec::new(),
        }
    }
    fn fail(&mut self, step: PoleStep, failure: Fail) {
        self.failures.push((step, failure));
    }
    fn failure(&mut self, step: PoleStep) -> Option<Fail> {
        if self.failures.first().is_some_and(|(next, _)| *next == step) {
            Some(self.failures.remove(0).1)
        } else {
            None
        }
    }
}

impl super::provider_test::pole::PoleEffects for PoleMock {
    fn unseal(&mut self, _: usize) -> Result<PieToken, Fail> {
        if let Some(failure) = self.failure(PoleStep::Unseal) {
            return Err(failure);
        }
        let token = PieToken::mint(1);
        self.root = Some(PieInfo {
            token,
            kind: PieKind::Pole,
            permission: Permission::FETCH | Permission::VEST,
            owner: TaskId::new(8),
            vestor: TaskId::new(0),
            mark: Mark::of("image"),
            alive: true,
        });
        Ok(token)
    }
    fn open(&mut self, _: PieToken) -> Result<(usize, usize), Fail> {
        if let Some(failure) = self.failure(PoleStep::Open) {
            return Err(failure);
        }
        self.mapped = true;
        Ok((0x1000, env::PAGE_SIZE))
    }
    fn write(&mut self, _: usize, _: &[u8]) -> Result<(), Fail> {
        self.failure(PoleStep::Write).map_or(Ok(()), Err)
    }
    fn shut(&mut self, _: PieToken) -> Result<(), Fail> {
        if let Some(failure) = self.failure(PoleStep::Shut) {
            return Err(failure);
        }
        self.mapped = false;
        Ok(())
    }
    fn inspect(&mut self, _: PieToken) -> Result<PieInfo, Fail> {
        if let Some(failure) = self.failure(PoleStep::Inspect) {
            return Err(failure);
        }
        self.root.ok_or(Fail::SourceLost)
    }
    fn release(&mut self, _: PieToken) -> Result<(), Fail> {
        if let Some(failure) = self.failure(PoleStep::Release) {
            return Err(failure);
        }
        self.root.as_mut().ok_or(Fail::SourceLost)?.alive = false;
        Ok(())
    }
}

fn build_pole(
    effects: &mut PoleMock,
) -> Result<
    super::provider_test::pole::PoleResource<fn(&super::provider_test::LeaseRequest) -> bool>,
    super::provider_test::pole::InitError<fn(&super::provider_test::LeaseRequest) -> bool>,
> {
    super::provider_test::pole::PoleResource::from_bytes_with(
        super::provider_test::pole::PoleSpec {
            key: Key::of("vendor/image"),
            bytes: b"payload",
            description: b"descriptor",
            admission: |_| true,
        },
        effects,
    )
}

#[test]
fn repeated_request_reuses_pending_and_parameter_changes_conflict() {
    let mut book = Ledger::new();
    let caller = origin(4, 90);
    let params = parameters(Permission::FETCH);
    assert_eq!(
        book.begin(LeaseStart {
            key: LeaseKey {
                id: LeaseId(1),
                origin: caller
            },
            parameters: params
        }),
        Ok(Begin::Started)
    );
    assert_eq!(
        book.begin(LeaseStart {
            key: LeaseKey {
                id: LeaseId(1),
                origin: caller
            },
            parameters: params
        }),
        Ok(Begin::Existing(State::Acquiring, None, None))
    );
    assert_eq!(
        book.begin(LeaseStart {
            key: LeaseKey {
                id: LeaseId(1),
                origin: caller
            },
            parameters: parameters(Permission::STORE)
        }),
        Err(Fail::Conflict)
    );
    assert_eq!(
        book.begin(LeaseStart {
            key: LeaseKey {
                id: LeaseId(1),
                origin: origin(5, 90)
            },
            parameters: params
        }),
        Ok(Begin::Started)
    );
    assert_eq!(
        book.begin(LeaseStart {
            key: LeaseKey {
                id: LeaseId(1),
                origin: origin(4, 91)
            },
            parameters: params
        }),
        Ok(Begin::Started)
    );
}

#[test]
fn cancel_pending_or_unseen_closes_the_id_and_old_delivery_cannot_reopen_it() {
    let mut book = Ledger::new();
    let caller = origin(4, 90);
    let params = parameters(Permission::FETCH);
    assert_eq!(
        book.begin(LeaseStart {
            key: LeaseKey {
                id: LeaseId(1),
                origin: caller
            },
            parameters: params
        }),
        Ok(Begin::Started)
    );
    assert_eq!(book.cancel(LeaseId(1), caller), Ok(State::Releasing));
    assert_eq!(book.finish_release(LeaseId(1), caller), Ok(State::Closed));
    assert_eq!(
        book.begin(LeaseStart {
            key: LeaseKey {
                id: LeaseId(1),
                origin: caller
            },
            parameters: params
        }),
        Ok(Begin::Existing(State::Closed, None, None))
    );

    assert_eq!(book.cancel(LeaseId(3), caller), Ok(State::Closed));
    assert_eq!(
        book.inspect(LeaseId(3), caller),
        Ok((State::Closed, None, None))
    );
    assert_eq!(
        book.begin(LeaseStart {
            key: LeaseKey {
                id: LeaseId(3),
                origin: caller
            },
            parameters: params
        }),
        Err(Fail::Conflict)
    );
    assert_eq!(
        book.begin(LeaseStart {
            key: LeaseKey {
                id: LeaseId(2),
                origin: caller
            },
            parameters: params
        }),
        Err(Fail::Conflict)
    );
}

#[test]
fn caller_and_authority_entry_scope_ids_and_inspection() {
    let mut book = Ledger::new();
    let params = parameters(Permission::FETCH);
    let a = origin(4, 90);
    let b = origin(4, 91);
    assert_eq!(
        book.begin(LeaseStart {
            key: LeaseKey {
                id: LeaseId(1),
                origin: a
            },
            parameters: params
        }),
        Ok(Begin::Started)
    );
    assert_eq!(book.inspect(LeaseId(1), b), Err(Fail::Unknown));
    assert_eq!(
        book.begin(LeaseStart {
            key: LeaseKey {
                id: LeaseId(1),
                origin: b
            },
            parameters: params
        }),
        Ok(Begin::Started)
    );
    assert_eq!(
        book.begin(LeaseStart {
            key: LeaseKey {
                id: LeaseId(2),
                origin: a
            },
            parameters: params
        }),
        Ok(Begin::Started)
    );
    assert_eq!(
        book.begin(LeaseStart {
            key: LeaseKey {
                id: LeaseId(2),
                origin: b
            },
            parameters: params
        }),
        Ok(Begin::Started)
    );
}

#[test]
fn remote_requester_stays_distinct_from_the_local_provider_actor() {
    let provider = TaskId::new(8);
    let requester = TaskId::new(44);
    let bound = Origin::bind(OriginBinding {
        provider,
        actor: provider,
        requester,
        authority: 90,
    })
    .unwrap();
    assert_eq!(bound.requester, requester);
    assert_eq!(bound.authority, 90);
    assert_eq!(
        Origin::bind(OriginBinding {
            provider,
            actor: TaskId::new(9),
            requester,
            authority: 90
        }),
        Err(Fail::Denied)
    );
    assert_eq!(
        Origin::bind(OriginBinding {
            provider,
            actor: provider,
            requester: TaskId::new(0),
            authority: 90
        }),
        Err(Fail::Denied)
    );
}

#[test]
fn capacity_is_bounded_and_exhaustion_is_reported() {
    let mut book = Ledger::new();
    let caller = origin(4, 90);
    for id in 1..=system_api::supply::MAX_LEASES as u64 {
        assert_eq!(
            book.begin(LeaseStart {
                key: LeaseKey {
                    id: LeaseId(id),
                    origin: caller
                },
                parameters: parameters(Permission::FETCH)
            }),
            Ok(Begin::Started)
        );
    }
    assert_eq!(
        book.begin(LeaseStart {
            key: LeaseKey {
                id: LeaseId(257),
                origin: caller
            },
            parameters: parameters(Permission::FETCH)
        }),
        Err(Fail::Full)
    );
    assert_eq!(
        book.begin(LeaseStart {
            key: LeaseKey {
                id: LeaseId(257),
                origin: caller
            },
            parameters: parameters(Permission::FETCH)
        }),
        Err(Fail::Full)
    );
}

#[test]
fn caller_watermark_capacity_is_explicit_and_bounded() {
    let mut book = Ledger::new();
    for requester in 1..=super::lease::MAX_CALLERS as u64 {
        let origin = Origin {
            requester: TaskId::new(requester as usize),
            authority: 90,
        };
        assert_eq!(
            book.begin(LeaseStart {
                key: LeaseKey {
                    id: LeaseId(1),
                    origin
                },
                parameters: parameters(Permission::FETCH)
            }),
            Ok(Begin::Started)
        );
    }
    let overflow = Origin {
        requester: TaskId::new((super::lease::MAX_CALLERS + 1) as usize),
        authority: 90,
    };
    assert_eq!(
        book.begin(LeaseStart {
            key: LeaseKey {
                id: LeaseId(1),
                origin: overflow
            },
            parameters: parameters(Permission::FETCH)
        }),
        Err(Fail::Full)
    );
    assert_eq!(book.inspect(LeaseId(1), overflow), Err(Fail::Unknown));
}

#[test]
fn closed_leases_are_compacted_without_reopening_the_latest_id() {
    let mut book = Ledger::new();
    let caller = origin(4, 90);
    let params = parameters(Permission::FETCH);
    for id in 1..=700 {
        assert_eq!(
            book.begin(LeaseStart {
                key: LeaseKey {
                    id: LeaseId(id),
                    origin: caller
                },
                parameters: params
            }),
            Ok(Begin::Started)
        );
        assert_eq!(
            book.release_begin(LeaseId(id), caller),
            Ok(State::Releasing)
        );
        assert_eq!(book.finish_release(LeaseId(id), caller), Ok(State::Closed));
        assert_eq!(
            book.inspect(LeaseId(id), caller),
            Ok((State::Closed, None, None))
        );
    }
    assert_eq!(
        book.begin(LeaseStart {
            key: LeaseKey {
                id: LeaseId(700),
                origin: caller
            },
            parameters: params
        }),
        Ok(Begin::Existing(State::Closed, None, None))
    );
    assert_eq!(
        book.begin(LeaseStart {
            key: LeaseKey {
                id: LeaseId(700),
                origin: caller
            },
            parameters: parameters(Permission::STORE)
        }),
        Err(Fail::Conflict)
    );
    assert_eq!(
        book.begin(LeaseStart {
            key: LeaseKey {
                id: LeaseId(701),
                origin: caller
            },
            parameters: params
        }),
        Ok(Begin::Started)
    );
}

#[test]
fn config_rejects_missing_sources_bad_permission_and_key_collisions() {
    use system_api::supply::Config;
    assert_eq!(
        system_api::supply::validate(
            &Config {
                bindings: &VALID_BINDINGS
            },
            &ONE_SOURCE
        ),
        Ok(())
    );
    assert_eq!(
        system_api::supply::validate(
            &Config {
                bindings: &VALID_BINDINGS
            },
            &NO_SOURCES
        ),
        Err(system_api::supply::ConfigError::MissingSource)
    );
    assert_eq!(
        system_api::supply::validate(
            &Config {
                bindings: &DENIED_BINDINGS
            },
            &ONE_SOURCE
        ),
        Err(system_api::supply::ConfigError::BadPermission)
    );
    assert_eq!(
        system_api::supply::validate(
            &Config {
                bindings: &VALID_BINDINGS
            },
            &DUPLICATE_SOURCES
        ),
        Err(system_api::supply::ConfigError::Collision)
    );
}

#[derive(Default)]
struct PieModel {
    next: usize,
    entries: Vec<(PieInfo, Option<PieToken>)>,
    fail_revoke: bool,
    /// 每次 `accord` 收到的窗口请求，按调用顺序；用来核“窗口真的从供给层走到了效果层”。
    windows: Vec<Option<(usize, usize)>>,
}

impl PieModel {
    fn seed(&mut self, info: PieInfo) -> PieToken {
        self.next = self.next.max(info.token.get());
        self.entries.push((info, None));
        info.token
    }

    fn descends_from(&self, root: PieToken, mut token: PieToken) -> bool {
        loop {
            let Some((_, Some(parent))) = self.entries.iter().find(|(info, _)| info.token == token)
            else {
                return false;
            };
            if *parent == root {
                return true;
            }
            token = *parent;
        }
    }

    fn issue(
        &mut self,
        token: PieToken,
        target: TaskId,
        permission: Permission,
        mark: Mark,
    ) -> Result<PieToken, Fail> {
        let source = self.inspect(token)?;
        if !source.alive || !source.permission.contains(permission) {
            return Err(Fail::Denied);
        }
        self.next += 1;
        let child = PieToken::mint(self.next);
        self.entries.push((
            PieInfo {
                token: child,
                kind: source.kind,
                permission,
                owner: source.owner,
                vestor: target,
                mark,
                alive: true,
            },
            Some(token),
        ));
        Ok(child)
    }

    fn alive_count(&self) -> usize {
        self.entries.iter().filter(|(entry, _)| entry.alive).count()
    }
}

impl PieEffects for PieModel {
    fn inspect(&mut self, token: PieToken) -> Result<PieInfo, Fail> {
        self.entries
            .iter()
            .find(|(info, _)| info.token == token)
            .map(|(info, _)| *info)
            .ok_or(Fail::SourceLost)
    }

    fn accord(&mut self, request: Accord) -> Result<PieToken, Fail> {
        self.windows.push(request.window);
        self.issue(
            request.token,
            request.target,
            request.permission,
            request.mark,
        )
    }

    fn revoke(&mut self, token: PieToken) -> Result<(), Fail> {
        if self.fail_revoke {
            self.fail_revoke = false;
            return Err(Fail::SourceLost);
        }
        let _ = self.inspect(token)?;
        let affected = self
            .entries
            .iter()
            .enumerate()
            .filter_map(|(at, (info, _))| {
                (info.token == token || self.descends_from(token, info.token)).then_some(at)
            })
            .collect::<Vec<_>>();
        for at in affected {
            self.entries[at].0.alive = false;
        }
        Ok(())
    }
}

#[derive(Clone)]
struct SharedPie(std::rc::Rc<std::cell::RefCell<PieModel>>);

impl SharedPie {
    fn fail_next_revoke(&self) {
        self.0.borrow_mut().fail_revoke = true;
    }
    fn alive_count(&self) -> usize {
        self.0.borrow().alive_count()
    }
}

impl PieEffects for SharedPie {
    fn inspect(&mut self, token: PieToken) -> Result<PieInfo, Fail> {
        self.0.borrow_mut().inspect(token)
    }
    fn accord(&mut self, request: Accord) -> Result<PieToken, Fail> {
        self.0.borrow_mut().issue(
            request.token,
            request.target,
            request.permission,
            request.mark,
        )
    }
    fn revoke(&mut self, token: PieToken) -> Result<(), Fail> {
        self.0.borrow_mut().revoke(token)
    }
}

impl super::production_source::PieEffects for SharedPie {
    fn inspect(&mut self, token: PieToken) -> Result<PieInfo, Fail> {
        self.0.borrow_mut().inspect(token)
    }
    fn accord(&mut self, request: super::production_source::Accord) -> Result<PieToken, Fail> {
        self.0.borrow_mut().issue(
            request.token,
            request.target,
            request.permission,
            request.mark,
        )
    }
    fn revoke(&mut self, token: PieToken) -> Result<(), Fail> {
        self.0.borrow_mut().revoke(token)
    }
}

struct TestResource {
    key: Key,
    shared: PieToken,
    effects: SharedPie,
    allowed: std::rc::Rc<std::cell::Cell<bool>>,
    acquisitions: std::rc::Rc<std::cell::Cell<usize>>,
    partial: std::rc::Rc<std::cell::Cell<bool>>,
    bad_descriptor: std::rc::Rc<std::cell::Cell<bool>>,
    root_count: usize,
}

impl super::production_source::Resource for TestResource {
    fn key(&self) -> Key {
        self.key
    }
    fn authorize(&self, request: &super::production_source::LeaseRequest) -> bool {
        self.allowed.get()
            && request.origin.requester.get() == 44
            && request.permission == Permission::FETCH
            && request.target.get() != 0
    }
    fn acquire(
        &mut self,
        request: &super::production_source::LeaseRequest,
    ) -> Result<super::production_source::Provision, super::production_source::ProvisionError> {
        self.acquisitions.set(self.acquisitions.get() + 1);
        if self.root_count > super::lease::MAX_ROOTS {
            return Err(super::production_source::ProvisionError::new(Fail::Full));
        }
        let mut roots = Vec::new();
        roots
            .try_reserve_exact(self.root_count)
            .map_err(|_| super::production_source::ProvisionError::new(Fail::Full))?;
        for index in 0..self.root_count {
            let root = super::production_source::PrivateRoot::derive_with(
                super::production_source::RootAssignment {
                    shared: self.shared,
                    index,
                    window: None,
                },
                super::production_source::RootRequest {
                    id: request.id,
                    origin: request.origin,
                    owner: request.owner,
                    permission: request.permission,
                },
                &mut self.effects,
            )
            .map_err(super::production_source::ProvisionError::new)?;
            roots.push(root);
        }
        if self.partial.get() {
            return Err(super::production_source::ProvisionError::with_roots(
                Fail::SourceLost,
                roots,
            ));
        }
        let mut provision = super::production_source::Provision::new(roots, 0, b"fixture")?;
        if self.bad_descriptor.get() {
            provision.description_len = u16::MAX;
        }
        Ok(provision)
    }
}

fn production_source(
    allowed: bool,
) -> (
    super::production_source::Source,
    SharedPie,
    std::rc::Rc<std::cell::Cell<usize>>,
    std::rc::Rc<std::cell::Cell<bool>>,
    std::rc::Rc<std::cell::Cell<bool>>,
    std::rc::Rc<std::cell::Cell<bool>>,
) {
    production_source_with_roots(allowed, 1)
}

fn production_source_with_roots(
    allowed: bool,
    root_count: usize,
) -> (
    super::production_source::Source,
    SharedPie,
    std::rc::Rc<std::cell::Cell<usize>>,
    std::rc::Rc<std::cell::Cell<bool>>,
    std::rc::Rc<std::cell::Cell<bool>>,
    std::rc::Rc<std::cell::Cell<bool>>,
) {
    let provider = TaskId::new(8);
    let mut model = PieModel::default();
    let authority = model.seed(PieInfo {
        token: PieToken::mint(90),
        kind: PieKind::Hole,
        permission: Permission::FETCH | Permission::STORE | Permission::VEST,
        owner: provider,
        vestor: TaskId::new(0),
        mark: system_api::supply::ASK,
        alive: true,
    });
    let shared = model.seed(PieInfo {
        token: PieToken::mint(1),
        kind: PieKind::Pole,
        permission: Permission::FETCH | Permission::VEST,
        owner: TaskId::new(31),
        vestor: TaskId::new(44),
        mark: Mark::of("shared"),
        alive: true,
    });
    let effects = SharedPie(std::rc::Rc::new(std::cell::RefCell::new(model)));
    let acquisitions = std::rc::Rc::new(std::cell::Cell::new(0));
    let admission = std::rc::Rc::new(std::cell::Cell::new(allowed));
    let partial = std::rc::Rc::new(std::cell::Cell::new(false));
    let bad_descriptor = std::rc::Rc::new(std::cell::Cell::new(false));
    let mut source =
        super::production_source::Source::new_with(super::production_source::SourceSetup {
            key: Key::of("vendor/source"),
            authority,
            owner: provider,
            effects: Box::new(effects.clone()),
        })
        .unwrap();
    source
        .register(Box::new(TestResource {
            key: Key::of("vendor/resource"),
            shared,
            effects: effects.clone(),
            allowed: admission.clone(),
            acquisitions: acquisitions.clone(),
            partial: partial.clone(),
            bad_descriptor: bad_descriptor.clone(),
            root_count,
        }))
        .unwrap();
    (
        source,
        effects,
        acquisitions,
        admission,
        partial,
        bad_descriptor,
    )
}

fn lease_request(op: u8, lease: u64, target: usize) -> system_api::supply::Request {
    system_api::supply::Request {
        op,
        lease,
        target: TaskId::new(target),
        source: Key::of("vendor/source").mark().get(),
        resource: Key::of("vendor/resource").mark().get(),
        slot: Mark::of("target/resource").get(),
        permission: Permission::FETCH.bits(),
        back: PieToken::NONE,
    }
}

fn test_handle(
    source: &mut super::production_source::Source,
    actor: TaskId,
    requester: TaskId,
    request: system_api::supply::Request,
) -> system_api::supply::Response {
    source.test_handle_from(super::production_source::Incoming {
        actor,
        requester,
        request,
    })
}

#[test]
fn production_private_root_revoke_covers_regrant_descendants_but_keeps_siblings() {
    let mut effects = PieModel::default();
    let shared = effects.seed(PieInfo {
        token: PieToken::mint(1),
        kind: PieKind::Pole,
        permission: Permission::FETCH | Permission::VEST,
        owner: TaskId::new(31),
        vestor: TaskId::new(44),
        mark: Mark::of("machine-root"),
        alive: true,
    });
    let request = |id| RootRequest {
        id: LeaseId(id),
        origin: origin(4, 90),
        owner: TaskId::new(8),
        permission: Permission::FETCH | Permission::VEST,
    };
    let mut lease_a = PrivateRoot::derive_with(
        RootAssignment {
            shared,
            index: 0,
            window: None,
        },
        request(1),
        &mut effects,
    )
    .unwrap();
    let lease_b = PrivateRoot::derive_with(
        RootAssignment {
            shared,
            index: 0,
            window: None,
        },
        request(2),
        &mut effects,
    )
    .unwrap();
    let lease_a_token = lease_a.token();
    let delivered = lease_a
        .deliver_with(
            DeliveryRequest {
                target: TaskId::new(80),
                slot: Mark::of("slot"),
                permission: Permission::FETCH | Permission::VEST,
                description: b"image",
            },
            &mut effects,
        )
        .unwrap();
    let leaf = effects
        .accord(Accord {
            token: delivered.seed,
            target: TaskId::new(81),
            permission: Permission::FETCH,
            mark: Mark::of("kept-grandchild"),
            window: None,
        })
        .unwrap();

    lease_a.revoke_with(&mut effects).unwrap();
    assert!(!effects.inspect(lease_a_token).unwrap().alive);
    assert!(!effects.inspect(delivered.seed).unwrap().alive);
    assert!(!effects.inspect(leaf).unwrap().alive);
    assert!(effects.inspect(lease_b.token()).unwrap().alive);
    assert!(effects.inspect(shared).unwrap().alive);
}

#[test]
fn production_private_root_derivation_carries_the_assignment_window_to_the_effect() {
    let mut effects = PieModel::default();
    let shared = effects.seed(PieInfo {
        token: PieToken::mint(1),
        kind: PieKind::Pole,
        permission: Permission::FETCH | Permission::VEST,
        owner: TaskId::new(31),
        vestor: TaskId::new(44),
        mark: Mark::of("machine-root"),
        alive: true,
    });
    let request = RootRequest {
        id: LeaseId(1),
        origin: origin(4, 90),
        owner: TaskId::new(8),
        permission: Permission::FETCH,
    };
    // `None` = 不请求窗口：效果层收到的就是 `None`（继承父分支那一段在内核那一侧）。
    PrivateRoot::derive_with(
        RootAssignment {
            shared,
            index: 0,
            window: None,
        },
        request,
        &mut effects,
    )
    .unwrap();
    assert_eq!(effects.windows.last().copied(), Some(None));
    // 真实管理者给的绝对页坐标原样走到效果层。
    PrivateRoot::derive_with(
        RootAssignment {
            shared,
            index: 1,
            window: Some((1, 3)),
        },
        request,
        &mut effects,
    )
    .unwrap();
    assert_eq!(effects.windows.last().copied(), Some(Some((1, 3))));
}

#[test]
fn production_source_handler_authorizes_before_claiming_origin_watermark_and_reuses_ids() {
    let (mut source, effects, acquisitions, admission, _, _) = production_source(false);
    let provider = TaskId::new(8);
    let requester = TaskId::new(44);

    let denied = test_handle(
        &mut source,
        provider,
        requester,
        lease_request(system_api::supply::ACQUIRE, 50, 80),
    );
    assert_eq!(denied.status, Fail::Denied.code());

    admission.set(true);
    let lost_reply = source.test_answer(
        super::production_source::Incoming {
            actor: provider,
            requester,
            request: lease_request(system_api::supply::ACQUIRE, 50, 80),
        },
        |response| {
            assert_eq!(response.status, wire::OK);
            Err(Fail::SourceLost)
        },
    );
    assert_eq!(lost_reply, Err(Fail::SourceLost));
    let granted = test_handle(
        &mut source,
        provider,
        requester,
        lease_request(system_api::supply::ACQUIRE, 50, 80),
    );
    assert_eq!(granted.status, wire::OK);
    assert_eq!(granted.state, State::Granted.code());
    assert_ne!(granted.seed, PieToken::NONE);
    assert_eq!(&granted.description[..granted.len as usize], b"fixture");
    assert_eq!(acquisitions.get(), 1);

    let replay = test_handle(
        &mut source,
        provider,
        requester,
        lease_request(system_api::supply::ACQUIRE, 50, 80),
    );
    assert_eq!(replay.status, wire::OK);
    assert_eq!(replay.seed, granted.seed);
    assert_eq!(acquisitions.get(), 1);

    let conflict = test_handle(
        &mut source,
        provider,
        requester,
        lease_request(system_api::supply::ACQUIRE, 50, 81),
    );
    assert_eq!(conflict.status, Fail::Conflict.code());
    assert_eq!(acquisitions.get(), 1);
    assert!(effects.0.borrow_mut().inspect(granted.seed).unwrap().alive);
}

#[test]
fn remote_source_uses_native_requester_and_provider_actor_without_joining_requester() {
    let (mut source, _, _, _, _, _) = production_source(true);
    let provider = TaskId::new(8);
    let requester = TaskId::new(44);
    let response = test_handle(
        &mut source,
        provider,
        requester,
        lease_request(system_api::supply::ACQUIRE, 73, 80),
    );
    assert_ne!(provider, requester);
    assert_eq!(response.status, wire::OK);
    assert_eq!(response.state, State::Granted.code());
    assert_ne!(response.seed, PieToken::NONE);
}

#[test]
fn production_source_recovers_partial_acquire_cleanup_and_rejects_bad_descriptor() {
    let (mut source, effects, acquisitions, _, partial, _) = production_source(true);
    let provider = TaskId::new(8);
    let requester = TaskId::new(44);
    partial.set(true);
    effects.fail_next_revoke();

    let failed = test_handle(
        &mut source,
        provider,
        requester,
        lease_request(system_api::supply::ACQUIRE, 60, 80),
    );
    assert_eq!(failed.status, Fail::SourceLost.code());
    assert_eq!(failed.state, State::Releasing.code());
    assert_eq!(
        effects.alive_count(),
        3,
        "failed revoke must retain the private root"
    );

    let retried = test_handle(
        &mut source,
        provider,
        requester,
        lease_request(system_api::supply::RELEASE, 60, 0),
    );
    assert_eq!(retried.status, wire::OK);
    assert_eq!(retried.state, State::Closed.code());
    assert_eq!(
        effects.alive_count(),
        2,
        "retry must revoke the partial lease root"
    );
    assert_eq!(acquisitions.get(), 1);

    let (mut source, effects, acquisitions, _, _, bad_descriptor) = production_source(true);
    bad_descriptor.set(true);
    let invalid = test_handle(
        &mut source,
        provider,
        requester,
        lease_request(system_api::supply::ACQUIRE, 61, 80),
    );
    assert_eq!(invalid.status, Fail::Bad.code());
    assert_eq!(invalid.state, State::Closed.code());
    assert_eq!(
        effects.alive_count(),
        2,
        "invalid descriptor must not orphan a derived root"
    );
    assert_eq!(acquisitions.get(), 1);
}

#[test]
fn production_source_handler_cancels_unseen_and_retries_failed_revoke() {
    let (mut source, effects, _, _, _, _) = production_source(true);
    let provider = TaskId::new(8);
    let requester = TaskId::new(44);
    let closed = test_handle(
        &mut source,
        provider,
        requester,
        lease_request(system_api::supply::CANCEL, 5, 0),
    );
    assert_eq!(closed.status, wire::OK);
    assert_eq!(closed.state, State::Closed.code());
    let late = test_handle(
        &mut source,
        provider,
        requester,
        lease_request(system_api::supply::ACQUIRE, 5, 80),
    );
    assert_eq!(late.status, Fail::Conflict.code());

    let granted = test_handle(
        &mut source,
        provider,
        requester,
        lease_request(system_api::supply::ACQUIRE, 6, 80),
    );
    assert_eq!(granted.status, wire::OK);
    effects.0.borrow_mut().fail_revoke = true;
    let failed = test_handle(
        &mut source,
        provider,
        requester,
        lease_request(system_api::supply::RELEASE, 6, 0),
    );
    assert_eq!(failed.status, Fail::SourceLost.code());
    assert_eq!(failed.state, State::Releasing.code());
    assert!(effects.0.borrow_mut().inspect(granted.seed).unwrap().alive);

    let released = test_handle(
        &mut source,
        provider,
        requester,
        lease_request(system_api::supply::RELEASE, 6, 0),
    );
    assert_eq!(released.status, wire::OK);
    assert_eq!(released.state, State::Released.code());
    assert!(!effects.0.borrow_mut().inspect(granted.seed).unwrap().alive);
}

#[test]
fn production_source_accepts_full_root_capacity_and_releases_every_root() {
    let (mut source, effects, _, _, _, _) =
        production_source_with_roots(true, super::lease::MAX_ROOTS);
    let provider = TaskId::new(8);
    let requester = TaskId::new(44);
    let granted = test_handle(
        &mut source,
        provider,
        requester,
        lease_request(system_api::supply::ACQUIRE, 101, 80),
    );
    assert_eq!(granted.status, wire::OK);
    assert_eq!(granted.state, State::Granted.code());
    assert_ne!(granted.seed, PieToken::NONE);
    assert_eq!(effects.alive_count(), 2 + super::lease::MAX_ROOTS + 1);

    let released = test_handle(
        &mut source,
        provider,
        requester,
        lease_request(system_api::supply::RELEASE, 101, 0),
    );
    assert_eq!(released.status, wire::OK);
    assert_eq!(released.state, State::Released.code());
    assert_eq!(
        effects.alive_count(),
        2,
        "release must revoke all private roots"
    );

    let (mut source, effects, _, _, _, _) =
        production_source_with_roots(true, super::lease::MAX_ROOTS + 1);
    let full = test_handle(
        &mut source,
        provider,
        requester,
        lease_request(system_api::supply::ACQUIRE, 102, 80),
    );
    assert_eq!(full.status, Fail::Full.code());
    assert_eq!(full.state, State::Closed.code());
    assert_eq!(effects.alive_count(), 2, "overflow must not derive roots");
}

#[test]
fn failed_native_root_cleanup_stays_releasing_until_retry_succeeds() {
    let mut effects = PieModel::default();
    let shared = effects.seed(PieInfo {
        token: PieToken::mint(1),
        kind: PieKind::Pole,
        permission: Permission::FETCH | Permission::VEST,
        owner: TaskId::new(31),
        vestor: TaskId::new(44),
        mark: Mark::of("shared"),
        alive: true,
    });
    let caller = origin(4, 90);
    let params = parameters(Permission::FETCH);
    let mut ledger = Ledger::new();
    assert_eq!(
        ledger.begin(LeaseStart {
            key: LeaseKey {
                id: LeaseId(1),
                origin: caller
            },
            parameters: params
        }),
        Ok(Begin::Started)
    );
    let root = PrivateRoot::derive_with(
        RootAssignment {
            shared,
            index: 0,
            window: None,
        },
        RootRequest {
            id: LeaseId(1),
            origin: caller,
            owner: TaskId::new(8),
            permission: Permission::FETCH,
        },
        &mut effects,
    )
    .unwrap();
    let root_token = root.token();
    ledger
        .abort_with_roots(
            LeaseKey {
                id: LeaseId(1),
                origin: caller,
            },
            FailedRoots {
                failure: Fail::SourceLost,
                roots: vec![root],
            },
        )
        .unwrap();

    effects.fail_revoke = true;
    let roots = ledger.roots_mut(LeaseId(1), caller).unwrap();
    assert_eq!(roots[0].revoke_with(&mut effects), Err(Fail::SourceLost));
    assert_eq!(roots[0].token(), root_token);
    assert_eq!(
        ledger.finish_release(LeaseId(1), caller),
        Err(Fail::Pending)
    );
    assert_eq!(
        ledger.inspect(LeaseId(1), caller),
        Ok((State::Releasing, None, Some(Fail::SourceLost)))
    );

    let roots = ledger.roots_mut(LeaseId(1), caller).unwrap();
    roots[0].revoke_with(&mut effects).unwrap();
    roots.retain(|root| root.token() != PieToken::NONE);
    assert_eq!(ledger.finish_release(LeaseId(1), caller), Ok(State::Closed));
    assert!(!effects.inspect(root_token).unwrap().alive);
}

#[test]
fn invalid_provision_keeps_derived_roots_available_for_recovery() {
    let mut effects = PieModel::default();
    let shared = effects.seed(PieInfo {
        token: PieToken::mint(1),
        kind: PieKind::Pole,
        permission: Permission::FETCH | Permission::VEST,
        owner: TaskId::new(31),
        vestor: TaskId::new(44),
        mark: Mark::of("shared"),
        alive: true,
    });
    let root = PrivateRoot::derive_with(
        RootAssignment {
            shared,
            index: 0,
            window: None,
        },
        RootRequest {
            id: LeaseId(1),
            origin: origin(4, 90),
            owner: TaskId::new(8),
            permission: Permission::FETCH,
        },
        &mut effects,
    )
    .unwrap();
    let token = root.token();
    let error = match super::lease::Provision::new(vec![root], 1, b"bad-index") {
        Ok(_) => panic!("invalid delivery root index was accepted"),
        Err(error) => error,
    };
    let (failure, mut roots) = error.into_parts();
    assert_eq!(failure, Fail::Bad);
    assert_eq!(roots.len(), 1);
    roots[0].revoke_with(&mut effects).unwrap();
    assert!(!effects.inspect(token).unwrap().alive);
}

#[test]
fn partial_provider_failure_keeps_earlier_derived_roots_for_cleanup() {
    let mut effects = PieModel::default();
    let shared = effects.seed(PieInfo {
        token: PieToken::mint(1),
        kind: PieKind::Pole,
        permission: Permission::FETCH | Permission::VEST,
        owner: TaskId::new(31),
        vestor: TaskId::new(44),
        mark: Mark::of("shared"),
        alive: true,
    });
    let caller = origin(4, 90);
    let params = parameters(Permission::FETCH);
    let mut ledger = Ledger::new();
    ledger
        .begin(LeaseStart {
            key: LeaseKey {
                id: LeaseId(1),
                origin: caller,
            },
            parameters: params,
        })
        .unwrap();
    let first = PrivateRoot::derive_with(
        RootAssignment {
            shared,
            index: 0,
            window: None,
        },
        RootRequest {
            id: LeaseId(1),
            origin: caller,
            owner: TaskId::new(8),
            permission: Permission::FETCH,
        },
        &mut effects,
    )
    .unwrap();
    let first_token = first.token();

    // A later provider step fails after an earlier private child was already created.
    let error = super::lease::ProvisionError::with_roots(Fail::Denied, vec![first]);
    let (failure, roots) = error.into_parts();
    assert_eq!(failure, Fail::Denied);
    ledger
        .abort_with_roots(
            LeaseKey {
                id: LeaseId(1),
                origin: caller,
            },
            FailedRoots { failure, roots },
        )
        .unwrap();
    {
        let roots = ledger.roots_mut(LeaseId(1), caller).unwrap();
        for root in roots.iter_mut() {
            root.revoke_with(&mut effects).unwrap();
        }
        roots.retain(|root| root.token() != PieToken::NONE);
    }
    assert_eq!(ledger.finish_release(LeaseId(1), caller), Ok(State::Closed));
    assert!(!effects.inspect(first_token).unwrap().alive);
}

#[test]
fn pole_initialization_failures_retain_and_recover_every_created_resource() {
    for (step, failure, should_have_root) in [
        (PoleStep::Unseal, Fail::Denied, false),
        (PoleStep::Open, Fail::SourceLost, true),
        (PoleStep::Write, Fail::Denied, true),
        (PoleStep::Shut, Fail::SourceLost, true),
    ] {
        let mut effects = PoleMock::new();
        effects.fail(step, failure);
        let error = match build_pole(&mut effects) {
            Err(error) => error,
            Ok(_) => panic!("injected initialization failure was ignored"),
        };
        assert_eq!(error.failure(), failure);
        assert_eq!(
            error.cleanup_failure(),
            None,
            "one-shot cleanup should succeed for {step:?}"
        );
        assert!(error.into_resource().is_none());
        if should_have_root {
            assert!(!effects.root.unwrap().alive);
            assert_eq!(effects.mapped, false);
        } else {
            assert!(effects.root.is_none());
        }
    }
}

#[test]
fn pole_init_error_preserves_original_and_cleanup_failures_until_retry() {
    let mut effects = PoleMock::new();
    effects.fail(PoleStep::Write, Fail::Denied);
    effects.fail(PoleStep::Shut, Fail::Pending);
    let mut error = match build_pole(&mut effects) {
        Err(error) => error,
        Ok(_) => panic!("injected write failure was ignored"),
    };
    assert_eq!(error.failure(), Fail::Denied);
    assert_eq!(error.cleanup_failure(), Some(Fail::Pending));
    assert!(effects.mapped);
    assert!(effects.root.unwrap().alive);

    effects.fail(PoleStep::Inspect, Fail::SourceLost);
    assert_eq!(
        error.retry_cleanup_with(&mut effects),
        Err(Fail::SourceLost)
    );
    assert_eq!(error.failure(), Fail::Denied);
    assert_eq!(error.cleanup_failure(), Some(Fail::SourceLost));
    assert!(!effects.mapped);
    assert!(effects.root.unwrap().alive);

    effects.fail(PoleStep::Release, Fail::Pending);
    assert_eq!(error.retry_cleanup_with(&mut effects), Err(Fail::Pending));
    assert_eq!(error.failure(), Fail::Denied);
    assert_eq!(error.cleanup_failure(), Some(Fail::Pending));
    assert!(effects.root.unwrap().alive);

    assert_eq!(error.retry_cleanup_with(&mut effects), Ok(()));
    assert_eq!(error.failure(), Fail::Denied);
    assert_eq!(error.cleanup_failure(), None);
    assert!(!effects.root.unwrap().alive);
    assert!(error.into_resource().is_none());
}

#[test]
fn bounded_delivery_wire_round_trips_and_rejects_overlong_responses() {
    use wire::Message;
    let mut said = system_api::supply::Response {
        status: wire::OK,
        state: State::Granted.code(),
        seed: PieToken::mint(0x1234),
        len: system_api::supply::MAX_DESCRIPTION as u16,
        description: [0xa5; system_api::supply::MAX_DESCRIPTION],
    };
    let mut bytes = [0; 140];
    assert_eq!(said.store(&mut bytes), Some(140));
    let read = system_api::supply::Response::fetch(&bytes).unwrap();
    assert_eq!(
        (read.status, read.state, read.seed, read.len),
        (said.status, said.state, said.seed, said.len)
    );
    assert_eq!(read.description, said.description);
    bytes[10..12].copy_from_slice(&129u16.to_le_bytes());
    assert!(system_api::supply::Response::fetch(&bytes).is_none());
    said.len = 129;
    assert_eq!(said.store(&mut bytes), None);
    said.len = 128;
    let mut extended = [0; 141];
    assert_eq!(said.store(&mut bytes), Some(140));
    extended[..140].copy_from_slice(&bytes);
    assert!(system_api::supply::Response::fetch(&extended).is_none());
}

#[test]
fn machine_inventory_round_trips_names_and_authority_metadata() {
    let items = [
        hub_api::InventoryItem {
            resource: env::Name::Page(env::Page::Dtb),
            owner: TaskId::new(8),
            kind: PieKind::Pole,
            permission: Permission::FETCH | Permission::VEST,
        },
        hub_api::InventoryItem {
            resource: env::Name::Trap(env::Trap::SupervisorExternal),
            owner: TaskId::new(8),
            kind: PieKind::Nole,
            permission: Permission::FETCH | Permission::VEST,
        },
    ];
    let inventory = hub_api::Inventory::of(&items).expect("valid machine inventory");
    let mut bytes = [0; 1 + 2 * 29];
    assert_eq!(inventory.store(&mut bytes), Some(bytes.len()));
    let decoded = hub_api::Inventory::fetch(&bytes).expect("well-formed inventory");
    assert_eq!(decoded, inventory);
    assert_eq!(decoded.item(1), Some(items[1]));
    assert_eq!(
        hub_api::inventory_slot(0),
        Some(Mark::new(0x4855_4200_0000_0000))
    );
    assert!(hub_api::inventory_slot(hub_api::INVENTORY_MAX - 1).is_some());
    assert_eq!(hub_api::inventory_slot(hub_api::INVENTORY_MAX), None);

    let mut trailing = [0; 1 + 2 * 29 + 1];
    trailing[..bytes.len()].copy_from_slice(&bytes);
    assert!(hub_api::Inventory::fetch(&trailing).is_none());
    let mut bad_kind = bytes;
    bad_kind[1 + 16 + 8] = PieKind::Tole as u8;
    assert!(hub_api::Inventory::fetch(&bad_kind).is_none());
}

#[test]
fn machine_inventory_accepts_its_full_bound_and_rejects_the_next_entry() {
    let mut items = Vec::new();
    for index in 0..hub_api::INVENTORY_MAX {
        items.push(hub_api::InventoryItem {
            resource: env::Name::Page(env::Page::Region((index as u64 + 1) * 0x1000)),
            owner: if index < 2 {
                TaskId::new(0)
            } else {
                TaskId::new(8)
            },
            kind: PieKind::Pole,
            permission: Permission::FETCH | Permission::VEST,
        });
    }
    assert_eq!(hub_api::Inventory::header(hub_api::INVENTORY_MAX), Some(64));
    assert!(hub_api::inventory_slot(hub_api::INVENTORY_MAX - 1).is_some());
    let full = hub_api::Inventory::of(&items).expect("full bounded inventory");
    let mut bytes = vec![0; hub_api::Inventory::encoded_len_for(hub_api::INVENTORY_MAX).unwrap()];
    assert_eq!(full.store(&mut bytes), Some(bytes.len()));
    assert_eq!(hub_api::Inventory::fetch(&bytes), Some(full));
    assert_eq!(full.item(0).unwrap().owner, TaskId::new(0));
    items.push(hub_api::InventoryItem {
        resource: env::Name::Page(env::Page::Region(0x1000_0000)),
        owner: TaskId::new(8),
        kind: PieKind::Pole,
        permission: Permission::FETCH | Permission::VEST,
    });
    assert_eq!(hub_api::Inventory::header(items.len()), None);
    assert!(hub_api::Inventory::of(&items).is_none());
}
