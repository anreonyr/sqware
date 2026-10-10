use ::resource::raw::{Hole, pies, reserve};
use env::pie;

pub const COMMAND: env::Mark = env::Mark::of("hierarchy-command");
pub const ANSWER: env::Mark = env::Mark::of("hierarchy-answer");
pub(crate) fn supply(task: env::TaskId) -> Result<(), &'static str> {
    use ::resource::port::{self, Access, Policy};
    let me = env::unit::self_id();
    for (mark, access) in [(COMMAND, Access::FETCH), (ANSWER, Access::STORE)] {
        let token = match ipc::session::establish::find(me, mark) {
            Ok(token) => token,
            Err(ipc::session::establish::DiscoveryFail::Missing) => {
                pie::unseal(env::UnsealArgs::hole(mark)).map_err(|_| "hierarchy fixture channel")?
            }
            Err(ipc::session::establish::DiscoveryFail::Ambiguous) => {
                return Err("hierarchy fixture channel ambiguous");
            }
        };
        port::ship(token, task, access, Policy::NONE).map_err(|_| "hierarchy fixture ship")?;
    }
    Ok(())
}

pub(crate) fn command(
    assembly: &mut crate::harness::probe::fixture::Fixture,
    task: env::TaskId,
    code: u8,
) {
    execute(assembly, task, (code, None));
}

fn reference_probe(
    assembly: &mut crate::harness::probe::fixture::Fixture,
    task: env::TaskId,
    entry: env::PieToken,
) {
    execute(assembly, task, (5, Some(entry)));
}

fn execute(
    assembly: &mut crate::harness::probe::fixture::Fixture,
    task: env::TaskId,
    request: (u8, Option<env::PieToken>),
) {
    let (code, reference) = request;
    use env::Wait;
    use env::wire::Span as _;
    use ipc::session::establish;
    let me = env::unit::self_id();
    let command = establish::find(me, COMMAND).unwrap();
    let answer = establish::find(me, ANSWER).unwrap();
    let receiver = assembly
        .resources
        .read::<crate::system::control::unit::Control>()
        .unwrap()
        .task("system-dependent")
        .unwrap();
    let seed = match reference {
        Some(entry) => {
            ::resource::port::ship(entry, receiver, env::Access::FETCH_STORE, env::Policy::VEST)
                .unwrap()
                .seed()
        }
        None => env::PieToken::NONE,
    };
    let mut bytes = [0u8; 17];
    bytes[0] = code;
    bytes[1..9].copy_from_slice(&(task.get() as u64).to_le_bytes());
    bytes[9..17].copy_from_slice(&seed.to_bytes());
    Hole::from_raw(command)
        .push(&bytes, Wait::AtMost(1000))
        .unwrap();
    let until = env::chrono::clock() + 10_000_000_000;
    loop {
        assembly.progress().expect("hierarchy progress");
        if code == 5 {
            use system_api::control::publication::Frame;
            use system_api::control::publication::Object;
            use system_api::control::publication::REF;
            use system_api::control::publication::Reply;
            assert_eq!(
                establish::find(me, REF),
                Err(establish::DiscoveryFail::Ambiguous)
            );
            let fake = reference.expect("explicit reference probe entry");
            let mut request = [0; Frame::LEN];
            if let Ok((n, _)) = Hole::from_raw(fake).pull(&mut request, Wait::POLL) {
                let frame = Frame::take(&request[..n]).unwrap();
                let wrong = env::TaskId::new(
                    assembly
                        .resources
                        .read::<crate::system::control::identity::Roster>()
                        .unwrap()
                        .authority()
                        .unwrap()
                        .get()
                        + 10000,
                );
                let reply = Reply::object(Object::Principal(
                    system_api::identity::PrincipalId::new(wrong, 0),
                ));
                let mut encoded = [0; Reply::LEN];
                let n = reply.store_at(&mut encoded, 0).unwrap();
                Hole::from_raw(frame.back)
                    .push(&encoded[..n], Wait::AtMost(1000))
                    .unwrap();
                let _ = pie::release(frame.back, env::ReleaseMode::Revoke);
            }
        }
        if let Ok((1, from)) = Hole::from_raw(answer).pull(&mut bytes, Wait::POLL) {
            assert_eq!(
                from,
                assembly
                    .resources
                    .read::<crate::system::control::unit::Control>()
                    .unwrap()
                    .task("system-dependent")
                    .unwrap()
            );
            assert_eq!(bytes[0], code);
            break;
        }
        assert!(
            env::chrono::clock() < until,
            "hierarchy worker did not finish command {code}"
        );
        execution::room::park(core::time::Duration::from_millis(1)).unwrap();
    }
}

pub(crate) fn exercise(
    assembly: &mut crate::harness::probe::fixture::Fixture,
    operator: &system_client::operator::Face,
    authority: env::TaskId,
    service: env::TaskId,
    target: env::TaskId,
) -> system_api::identity::CoalitionId {
    use env::Wait;
    use ipc::session::establish;
    use system_api::control::publication::Object;
    use system_api::identity::Grant;
    use system_api::identity::Install;
    use system_api::identity::Reply;
    use system_api::identity::Selector;
    use system_api::identity::Subject;
    use system_api::identity::Wire;
    use system_api::operator::Fail;
    use system_api::operator::Permit;
    use system_client::identity::Face;
    use system_client::identity::Installer;
    use system_client::identity::Organization;
    let wait = Wait::AtMost(3000);
    let entry = |g: Grant| establish::find(authority, g.mark()).unwrap();
    let face = |g| Face::direct(authority, g, entry(g)).unwrap();
    let me = env::unit::self_id();
    let Reply::Binding(Some(saved)) = face(Grant::Resolve).call(Wire::Resolve(me), wait).unwrap()
    else {
        panic!("control binding");
    };
    let Reply::Binding(Some(binding)) = face(Grant::Resolve)
        .call(Wire::Resolve(service), wait)
        .unwrap()
    else {
        panic!("resource service binding");
    };
    let p = binding.current.principal;
    let organization = Organization::direct(
        authority,
        entry(Grant::Found),
        entry(Grant::Admit),
        entry(Grant::Expel),
    )
    .unwrap();
    let coalition = organization.found(wait).unwrap();
    organization.admit(coalition, p, wait).unwrap();
    face(Grant::Activate)
        .call(
            Wire::Activate(
                service,
                system_api::identity::CoalitionSet::new(&[coalition]).unwrap(),
            ),
            wait,
        )
        .unwrap();
    face(Grant::Activate)
        .call(
            Wire::Activate(
                target,
                system_api::identity::CoalitionSet::new(&[coalition]).unwrap(),
            ),
            wait,
        )
        .unwrap();
    {
        {
            let registration = crate::system::publication::Registration {
                name: ("named-subject").into(),
                object: Object::Principal(p),
                lifetime: None,
            };
            crate::system::control::identity::validate(
                &assembly
                    .resources
                    .read::<crate::system::control::identity::Roster>()
                    .unwrap(),
                registration.object,
            )
            .map_err(|_| "identity alias source")
            .and_then(|_| {
                assembly
                    .resources
                    .write::<crate::system::publication::Names>()
                    .unwrap()
                    .register(
                        &mut assembly
                            .resources
                            .write::<crate::system::operator::management::Tree>()
                            .unwrap(),
                        registration,
                    )
            })
        }
        .unwrap();
        {
            let registration = crate::system::publication::Registration {
                name: ("named-league").into(),
                object: Object::Coalition(coalition),
                lifetime: None,
            };
            crate::system::control::identity::validate(
                &assembly
                    .resources
                    .read::<crate::system::control::identity::Roster>()
                    .unwrap(),
                registration.object,
            )
            .map_err(|_| "identity alias source")
            .and_then(|_| {
                assembly
                    .resources
                    .write::<crate::system::publication::Names>()
                    .unwrap()
                    .register(
                        &mut assembly
                            .resources
                            .write::<crate::system::operator::management::Tree>()
                            .unwrap(),
                        registration,
                    )
            })
        }
        .unwrap();
        {
            let registration = crate::system::publication::Registration {
                name: ("named-league").into(),
                object: Object::Coalition(coalition),
                lifetime: None,
            };
            crate::system::control::identity::validate(
                &assembly
                    .resources
                    .read::<crate::system::control::identity::Roster>()
                    .unwrap(),
                registration.object,
            )
            .map_err(|_| "identity alias source")
            .and_then(|_| {
                assembly
                    .resources
                    .write::<crate::system::publication::Names>()
                    .unwrap()
                    .register(
                        &mut assembly
                            .resources
                            .write::<crate::system::operator::management::Tree>()
                            .unwrap(),
                        registration,
                    )
            })
        }
        .unwrap();
        let other = organization.found(wait).unwrap();
        assert!(
            {
                let registration = crate::system::publication::Registration {
                    name: ("named-league").into(),
                    object: Object::Coalition(other),
                    lifetime: None,
                };
                crate::system::control::identity::validate(
                    &assembly
                        .resources
                        .read::<crate::system::control::identity::Roster>()
                        .unwrap(),
                    registration.object,
                )
                .map_err(|_| "identity alias source")
                .and_then(|_| {
                    assembly
                        .resources
                        .write::<crate::system::publication::Names>()
                        .unwrap()
                        .register(
                            &mut assembly
                                .resources
                                .write::<crate::system::operator::management::Tree>()
                                .unwrap(),
                            registration,
                        )
                })
            }
            .is_err()
        );
        assert!(
            {
                let registration = crate::system::publication::Registration {
                    name: ("bad/name").into(),
                    object: Object::Principal(p),
                    lifetime: None,
                };
                crate::system::control::identity::validate(
                    &assembly
                        .resources
                        .read::<crate::system::control::identity::Roster>()
                        .unwrap(),
                    registration.object,
                )
                .map_err(|_| "identity alias source")
                .and_then(|_| {
                    assembly
                        .resources
                        .write::<crate::system::publication::Names>()
                        .unwrap()
                        .register(
                            &mut assembly
                                .resources
                                .write::<crate::system::operator::management::Tree>()
                                .unwrap(),
                            registration,
                        )
                })
            }
            .is_err()
        );
        assembly
            .resources
            .write::<crate::system::publication::RuntimeNamespace>()
            .unwrap()
            .approve(crate::system::publication::RuntimeApproval {
                service,
                task: target,
                kind: "test".into(),
                name: "member".into(),
                permit: Permit::Identity(Selector::MemberOf(coalition)),
            })
            .unwrap();
        for i in 0..33 {
            assembly
                .resources
                .write::<crate::system::publication::RuntimeNamespace>()
                .unwrap()
                .approve(crate::system::publication::RuntimeApproval {
                    service,
                    task: target,
                    kind: "full".into(),
                    name: alloc::format!("f{i}"),
                    permit: Permit::Public,
                })
                .unwrap();
        }
        let source = pie::unseal(env::UnsealArgs::hole(env::Mark::of(
            "hierarchy-stale-condition",
        )))
        .unwrap();
        assembly
            .resources
            .write::<crate::system::publication::Publications>()
            .unwrap()
            .internal(
                &mut assembly
                    .resources
                    .write::<crate::system::operator::management::Tree>()
                    .unwrap(),
                &crate::system::publication::Internal {
                    road: (system_api::operator::path::Path::new("svc/fixtures/stale"))
                        .to_path_buf(),
                    entry: source,
                    access: (Permit::Identity(Selector::MemberOf(coalition)), me),
                },
            )
            .unwrap();
    }
    standalone_mutations(assembly);
    assert!(matches!(
        operator.root().tile(
            system_api::operator::path::Path::new("idt/principal/forged/ref"),
            wait
        ),
        Err(Fail::Unknown)
    ));
    sender_boundary(assembly);
    command(assembly, target, 1);
    let fake = pie::unseal(env::UnsealArgs::hole(system_api::control::publication::REF)).unwrap();
    reference_probe(assembly, target, fake);
    let _ = pie::seal(fake);
    let _ = pie::release(fake, env::ReleaseMode::Revoke);
    let service_road = assembly
        .resources
        .read::<crate::system::publication::RuntimeNamespace>()
        .unwrap()
        .runtime_road(service)
        .unwrap();
    let same = service_road
        .try_join("hole")
        .unwrap()
        .try_join("same-subject")
        .unwrap();
    assert_eq!(
        operator.tile(&same, wait).unwrap().token(wait),
        Err(Fail::Denied),
        "different principal must not obtain Exact resource"
    );
    let installer = Installer::direct(
        authority,
        entry(Grant::Bind),
        entry(Grant::Unbind),
        entry(Grant::Derive),
    )
    .unwrap();
    installer
        .bind(me, Install::Authorized(Subject::new(p, &[]).unwrap()), wait)
        .unwrap();
    assert!(
        operator.tile(&same, wait).unwrap().token(wait).is_ok(),
        "another Task representing the same principal must obtain it"
    );
    installer
        .bind(me, Install::Authorized(saved.current), wait)
        .unwrap();
    programs::debug::put(
        "hierarchy: runtime Exact permit allows another Task with same principal and denies different principal",
    );
    organization.expel(coalition, p, wait).unwrap();
    command(assembly, target, 2);
    let target_road = assembly
        .resources
        .read::<crate::system::publication::RuntimeNamespace>()
        .unwrap()
        .runtime_road(target)
        .unwrap();
    assembly
        .action(
            "system-child",
            crate::system::control::lifecycle::Action::Ruin,
        )
        .expect("hierarchy: scheduled ruin");
    assert!(env::unit::join_task(target, wait).unwrap_or(true));
    assembly.progress().unwrap();
    assert!(matches!(
        operator.root().tile(&target_road, wait),
        Err(Fail::Unknown)
    ));
    command(assembly, target, 3);
    assembly
        .action(
            "system-child",
            crate::system::control::lifecycle::Action::Mint,
        )
        .unwrap();
    let failed_task = assembly
        .resources
        .read::<crate::system::control::unit::Control>()
        .unwrap()
        .task("system-child")
        .unwrap();
    assembly
        .resources
        .write::<super::fixture::Fault>()
        .unwrap()
        .armed = true;
    assert!(
        assembly
            .action(
                "system-child",
                crate::system::control::lifecycle::Action::Embark {
                    parent: Some(service)
                }
            )
            .is_err()
    );
    let prepared = assembly
        .resources
        .read::<super::fixture::Fault>()
        .unwrap()
        .road
        .clone()
        .expect("injected failure must follow runtime preparation");
    assert!(
        assembly
            .resources
            .read::<crate::system::publication::RuntimeNamespace>()
            .unwrap()
            .runtime_road(failed_task)
            .is_none()
    );
    assert!(matches!(
        operator.root().tile(&prepared, wait),
        Err(Fail::Unknown)
    ));
    assert!(matches!(
        face(Grant::Resolve).call(Wire::Resolve(failed_task), wait),
        Ok(Reply::Binding(None))
    ));
    programs::debug::put(
        "hierarchy: failed launch after identity/runtime preparation compensates bind and directories",
    );
    coalition
}

pub fn codecs() {
    use env::wire::Span as _;
    use env::{PieToken, TaskId};
    use system_api::control::publication::Frame;
    use system_api::control::publication::Object;
    use system_api::control::publication::Reply;
    use system_api::control::publication::Scope;
    use system_api::control::publication::Target;
    use system_api::operator::EntryId;
    use system_api::operator::Permit;
    use system_api::operator::Tip;
    use system_api::operator::TipIn;
    let a = TaskId::new(77);
    let b = PieToken::from_bytes(&81u64.to_le_bytes()).unwrap();
    let p = system_api::identity::PrincipalId::new(a, 19);
    for target in [
        Target::Service {
            scope: Scope(1),
            group: "uart".into(),
            name: "rx".into(),
        },
        Target::IdentityName {
            object: Object::Principal(p),
            name: "named".into(),
        },
        Target::RuntimeResource {
            task: a,
            kind: "hole".into(),
            name: "entry".into(),
        },
    ] {
        let frame = Frame::new(1, target.clone(), (b, Permit::Public));
        let mut bytes = [0; Frame::LEN + 1];
        let n = frame.store_at(&mut bytes, 0).unwrap();
        assert_eq!(Frame::take(&bytes[..n]), Some(frame.clone()));
        assert_eq!(frame.target(), Some(target));
        assert!(Frame::take(&bytes[..n + 1]).is_none());
        for end in 0..n {
            assert!(Frame::take(&bytes[..end]).is_none());
        }
    }
    let mut bad = Frame::new(
        1,
        Target::Service {
            scope: Scope(4),
            group: "bad/name".into(),
            name: "x".into(),
        },
        (b, Permit::Public),
    );
    assert!(bad.target().is_none());
    bad.group = "legal".into();
    bad.kind = 99;
    assert!(bad.target().is_none());
    assert!(
        Reply::object(Object::Principal(p))
            .identity(TaskId::new(78))
            .is_err()
    );
    let mut bytes = [0; system_api::operator::TIP_LEN + 1];
    let tip = Tip::Plate {
        road: system_api::operator::path::Path::new("svc/test/entry").to_path_buf(),
        leaf: b,
        permit: Permit::Public,
        owner: a,
        replace: false,
        back: b,
    };
    let n = tip.store(&mut bytes).unwrap();
    assert!(
        matches!(TipIn::fetch(&bytes[..n]), Some(TipIn::Plate { owner, replace: false, .. }) if owner == a)
    );
    assert!(TipIn::fetch(&bytes[..n + 1]).is_none());
    for end in 0..n {
        assert!(TipIn::fetch(&bytes[..end]).is_none());
    }
    bytes[n - 9] = 2;
    assert!(TipIn::fetch(&bytes[..n]).is_none());
    for tip in [
        Tip::Abort {
            road: system_api::operator::path::Path::new("svc/test/entry").to_path_buf(),
            leaf: b,
            back: b,
        },
        Tip::Unplate {
            id: EntryId::new(79),
            back: b,
        },
        Tip::Empty {
            road: system_api::operator::path::Path::new("uit/1/2").to_path_buf(),
            back: b,
        },
    ] {
        let n = tip.store(&mut bytes).unwrap();
        assert!(TipIn::fetch(&bytes[..n]).is_some());
        for end in 0..n {
            assert!(TipIn::fetch(&bytes[..end]).is_none());
        }
    }
    programs::debug::put(
        "hierarchy: typed publication/ref/private mount codecs reject malformed and truncated frames",
    );
}

pub fn reference_lifetime() {
    use ::resource::port::{self, Access, Policy};
    use alloc::sync::Arc;
    use core::sync::atomic::{AtomicUsize, Ordering};
    use env::{Mark, Wait};
    let me = env::unit::self_id();
    let source = pie::unseal(env::UnsealArgs::hole(Mark::of("forget-source"))).unwrap();
    assert!(
        pie::release(source, env::ReleaseMode::Keep).is_err(),
        "original resource ownership cannot be forgotten"
    );
    let borrowed = port::ship(source, me, Access::FETCH | Access::STORE, Policy::VEST)
        .unwrap()
        .seed();
    let child = port::ship(borrowed, me, Access::FETCH | Access::STORE, Policy::VEST)
        .unwrap()
        .seed();
    assert_eq!(pie::same(source, child), Ok(true));
    let other = pie::unseal(env::UnsealArgs::hole(Mark::of("forget-source"))).unwrap();
    assert_eq!(pie::same(source, other), Ok(false));
    pie::release(borrowed, env::ReleaseMode::Keep).unwrap();
    Hole::from_raw(child).push(b"ok", Wait::POLL).unwrap();
    let mut bytes = [0; 2];
    assert_eq!(
        Hole::from_raw(source).pull(&mut bytes, Wait::POLL).unwrap(),
        (2, me)
    );
    pie::revoke(me, child).unwrap();
    assert!(
        reserve(child).is_err(),
        "reparented capability remains revocable upstream"
    );
    for round in 0..8 {
        let root = pie::unseal(env::UnsealArgs::hole(Mark::of("forget-race"))).unwrap();
        let middle = port::ship(root, me, Access::FETCH | Access::STORE, Policy::VEST)
            .unwrap()
            .seed();
        let stage = Arc::new(AtomicUsize::new(0));
        let progress = stage.clone();
        let root_id = me.get();
        let worker = execution::unit::task::spawn(move || {
            let middle = ipc::session::establish::claim(
                env::TaskId::new(root_id),
                Mark::of("forget-race"),
                Wait::AtMost(1000),
            )
            .unwrap();
            let me = env::unit::self_id();
            progress.store(1, Ordering::Release);
            while progress.load(Ordering::Acquire) < 2 {
                execution::room::park(core::time::Duration::from_millis(1)).unwrap();
            }
            for _ in 0..32 {
                port::ship(middle, me, Access::STORE, Policy::NONE).unwrap();
            }
            progress.store(3, Ordering::Release);
            while progress.load(Ordering::Acquire) < 4 {
                execution::room::park(core::time::Duration::from_millis(1)).unwrap();
            }
            assert!(
                !pies().any(|p| p.mark == Mark::of("forget-race")),
                "revocation missed a concurrent descendant"
            );
        });
        let downstream = port::ship(
            middle,
            worker.id(),
            Access::FETCH | Access::STORE,
            Policy::VEST,
        )
        .unwrap()
        .seed();
        while stage.load(Ordering::Acquire) < 1 {
            execution::room::park(core::time::Duration::from_millis(1)).unwrap();
        }
        stage.store(2, Ordering::Release);
        pie::release(middle, env::ReleaseMode::Keep).unwrap();
        while stage.load(Ordering::Acquire) < 3 {
            execution::room::park(core::time::Duration::from_millis(1)).unwrap();
        }
        if round % 2 == 0 {
            pie::revoke(worker.id(), downstream).unwrap();
        } else {
            pie::release(root, env::ReleaseMode::Revoke).unwrap();
        }
        stage.store(4, Ordering::Release);
        worker.join();
        let _ = pie::seal(root);
        let _ = pie::release(root, env::ReleaseMode::Revoke);
    }
    let _ = pie::seal(source);
    let _ = pie::release(source, env::ReleaseMode::Revoke);
    let _ = pie::seal(other);
    let _ = pie::release(other, env::ReleaseMode::Revoke);
    programs::debug::put(
        "hierarchy: Forget preserves delivered capabilities, Same distinguishes objects, concurrent Accord keeps upstream revoke/release effective",
    );
}

fn sender_boundary(assembly: &mut crate::harness::probe::fixture::Fixture) {
    use ::resource::port::{self, Access, Policy};
    use alloc::sync::Arc;
    use core::sync::atomic::{AtomicBool, Ordering};
    use env::Wait;
    use system_api::control::publication::ENTRY;
    use system_api::control::publication::Frame;
    use system_api::control::publication::Scope;
    use system_api::control::publication::Target;
    use system_api::operator::Fail;
    use system_api::operator::Permit;
    use system_client::control::publication::Client;
    let done = Arc::new(AtomicBool::new(false));
    let complete = done.clone();
    let control = env::unit::self_id().get();
    let protected = pie::unseal(env::UnsealArgs::hole(env::Mark::of("source-validation"))).unwrap();
    let raw = protected.get();
    let caller = execution::unit::task::spawn(move || {
        let control = env::TaskId::new(control);
        let entry = ipc::session::establish::claim(control, ENTRY, Wait::AtMost(1000)).unwrap();
        let client = Client::direct(control, entry).unwrap();
        let target = Target::Service {
            scope: Scope(4),
            group: "operator-fixture".into(),
            name: "entry".into(),
        };
        let source = pie::unseal(env::UnsealArgs::hole(env::Mark::of("publication-test"))).unwrap();
        assert_eq!(
            client.publish(target.clone(), source, Permit::Public, Wait::AtMost(3000)),
            Err(Fail::Denied),
            "holding the entry does not install a trusted publisher record"
        );
        let other = env::PieToken::from_bytes(&(raw as u64).to_le_bytes()).unwrap();
        let sender = ipc::rpc::request::Sender::<system_api::control::publication::Call>::from_raw(
            entry,
            system_api::control::publication::Call::BACK,
        )
        .unwrap();
        let raw_result = sender
            .call(ipc::time::Deadline::new(Wait::AtMost(3000)), |back| {
                let mut request = Frame::new(1, target, (other, Permit::Public));
                request.back = back;
                request
            })
            .unwrap()
            .result();
        assert_eq!(
            raw_result,
            Err(Fail::Denied),
            "source reference must come from the actual sender"
        );
        complete.store(true, Ordering::Release);
    });
    let entry = assembly
        .resources
        .read::<crate::system::publication::Entry>()
        .unwrap()
        .0;
    port::ship(entry, caller.id(), Access::STORE, Policy::NONE).unwrap();
    let until = env::chrono::clock() + 10_000_000_000;
    while !done.load(Ordering::Acquire) {
        assembly.progress().unwrap();
        assert!(env::chrono::clock() < until, "sender boundary timeout");
        execution::room::park(core::time::Duration::from_millis(1)).unwrap();
    }
    caller.join();
    assert!(
        reserve(protected).is_ok(),
        "invalid request must not discard somebody else's reference"
    );
    let _ = pie::seal(protected);
    let _ = pie::release(protected, env::ReleaseMode::Revoke);
    programs::debug::put(
        "hierarchy: real kernel sender and transferred source checks reject unregistered caller and forged source",
    );
}

fn standalone_mutations(assembly: &mut crate::harness::probe::fixture::Fixture) {
    use ::resource::port::{self, Access, Policy};
    use alloc::sync::Arc;
    use core::sync::atomic::{AtomicBool, Ordering};
    use env::Wait;
    use ipc::session::{Session, establish};
    use system_api::operator::EntryId;
    use system_api::operator::Fail;
    use system_api::operator::Where;
    use system_client::operator;
    use system_client::operator::Face;
    let control = env::unit::self_id();
    let host = assembly
        .resources
        .read::<crate::system::operator::management::Tree>()
        .unwrap()
        .host()
        .unwrap();
    for part in [true, false] {
        let done = Arc::new(AtomicBool::new(false));
        let complete = done.clone();
        let released = Arc::new(AtomicBool::new(false));
        let gate = released.clone();
        let root = control.get();
        let caller = execution::unit::task::spawn(move || {
            while !gate.load(Ordering::Acquire) {
                execution::room::park(core::time::Duration::from_millis(1)).unwrap();
            }
            let control = env::TaskId::new(root);
            let session = Session::open(control, operator::BERTH, Wait::AtMost(3000))
                .unwrap_or_else(|_| panic!("standalone Operator session"));
            let tree = Face::of(session);
            match part {
                true => assert_eq!(
                    tree.part(Where::Root, "idt".into(), Wait::AtMost(3000)),
                    Err(Fail::Denied)
                ),
                false => assert_eq!(
                    tree.trim(EntryId::new(0), Wait::AtMost(3000)),
                    Err(Fail::Denied)
                ),
            }
            let private = establish::claim(
                env::TaskId::new(host.get()),
                system_api::operator::TIP_MARK,
                Wait::AtMost(1000),
            )
            .unwrap();
            let tip = system_api::operator::Tip::Plate {
                road: system_api::operator::path::Path::new("idt/principal/forged/ref")
                    .to_path_buf(),
                leaf: env::PieToken::NONE,
                permit: system_api::operator::Permit::Public,
                owner: control,
                replace: false,
                back: env::PieToken::NONE,
            };
            let mut bytes = [0; system_api::operator::TIP_LEN];
            let n = tip.store(&mut bytes).unwrap();
            Hole::from_raw(private)
                .push(&bytes[..n], Wait::AtMost(1000))
                .unwrap();
            complete.store(true, Ordering::Release);
        });
        assembly
            .resources
            .read::<crate::system::control::identity::Roster>()
            .unwrap()
            .inherit(caller.id(), control)
            .unwrap();
        let private = establish::find(host, system_api::operator::TIP_MARK).unwrap();
        port::ship(private, caller.id(), Access::STORE, Policy::NONE).unwrap();
        released.store(true, Ordering::Release);
        let until = env::chrono::clock() + 10_000_000_000;
        while !done.load(Ordering::Acquire) {
            crate::system::operator::connect_task(&assembly.resources, caller.id()).unwrap();
            assembly.progress().unwrap();
            assert!(
                env::chrono::clock() < until,
                "standalone raw mutation timeout"
            );
            execution::room::park(core::time::Duration::from_millis(1)).unwrap();
        }
        let caller_id = caller.id();
        caller.join();
        assembly
            .resources
            .read::<crate::system::control::identity::Roster>()
            .unwrap()
            .unbind(caller_id)
            .unwrap();
    }
    programs::debug::put(
        "hierarchy: standalone Part/Trim deny bound same-subject callers; private Plate still requires actual Control sender",
    );
}
