pub const COMMAND: env::Mark = env::Mark::of("hierarchy-command");
pub const ANSWER: env::Mark = env::Mark::of("hierarchy-answer");
pub(crate) fn supply(task: env::TaskId) -> Result<(), &'static str> {
    use runtime::core::res::port::{self, Access, Policy};
    use runtime::env::mail::{self, HolePie};
    let me = runtime::env::unit::self_id();
    for (mark, access) in [(COMMAND, Access::FETCH), (ANSWER, Access::STORE)] {
        let token = protocol::communication::session::establish::find(me, mark)
            .or_else(|| mail::unseal_hole(mark).ok())
            .ok_or("hierarchy fixture channel")?;
        port::ship(&HolePie::from_token(token), task, access, Policy::NONE)
            .map_err(|_| "hierarchy fixture ship")?;
    }
    Ok(())
}

pub(crate) fn command(assembly: &mut crate::harness::probe::fixture::Fixture, task: env::TaskId, code: u8) {
    use env::Wait;
    use env::wire::Span as _;
    use protocol::communication::session::establish;
    use runtime::env::mail::HolePie;
    let me = runtime::env::unit::self_id();
    let command = establish::find(me, COMMAND).unwrap();
    let answer = establish::find(me, ANSWER).unwrap();
    let mut bytes = [code; 9];
    bytes[1..].copy_from_slice(&(task.get() as u64).to_le_bytes());
    HolePie::from_token(command)
        .push(&bytes, Wait::AtMost(1000))
        .unwrap();
    let until = runtime::env::chrono::clock() + 10_000_000_000;
    loop {
        assembly.progress()
            .expect("hierarchy progress");
        if code == 5 {
            use protocol::system::control::publication::{Frame, Object, REF, Reply};
            let fake = establish::find(me, REF).unwrap();
            let mut request = [0; Frame::LEN];
            if let Ok((n, _)) = HolePie::from_token(fake).pull(&mut request, Wait::POLL) {
                let frame = Frame::take(&request[..n]).unwrap();
                let wrong =
                    env::TaskId::new(assembly.roster.authority().unwrap().get() + 10000);
                let reply = Reply::object(Object::Principal(
                    protocol::system::identity::PrincipalId::new(wrong, 0),
                ));
                let mut encoded = [0; Reply::LEN];
                let n = reply.store_at(&mut encoded, 0).unwrap();
                HolePie::from_token(frame.back)
                    .push(&encoded[..n], Wait::AtMost(1000))
                    .unwrap();
                let _ = runtime::env::mail::release(frame.back);
            }
        }
        if let Ok((1, from)) = HolePie::from_token(answer).pull(&mut bytes, Wait::POLL) {
            assert_eq!(
                from,
                assembly.control.task("system-dependent").unwrap()
            );
            assert_eq!(bytes[0], code);
            break;
        }
        assert!(
            runtime::env::chrono::clock() < until,
            "hierarchy worker did not finish command {code}"
        );
        runtime::env::room::sleep(core::time::Duration::from_millis(1)).unwrap();
    }
}

pub(crate) fn exercise(
    assembly: &mut crate::harness::probe::fixture::Fixture,
    operator: &protocol::system::operator::client::Face,
    authority: env::TaskId,
    service: env::TaskId,
    target: env::TaskId,
) -> protocol::system::identity::CoalitionId {
    use env::Wait;
    use protocol::communication::session::establish;
    use protocol::system::identity::client::{Face, Installer, Organization};
    use protocol::system::identity::{Grant, Install, Reply, Selector, Subject, Wire};
    use protocol::system::operator::{Fail, Permit};
    use protocol::system::control::publication::Object;
    use runtime::env::mail::{self, HolePie};
    let wait = Wait::AtMost(3000);
    let entry = |g: Grant| establish::find(authority, g.mark()).unwrap();
    let face = |g| Face::direct(authority, g, entry(g)).unwrap();
    let me = runtime::env::unit::self_id();
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
    assembly.roster
        .activate(service, coalition)
        .unwrap();
    assembly.roster.activate(target, coalition).unwrap();
    {
        assembly.names.register(
                &assembly.roster,
                &mut assembly.tree,
                "named-subject",
                Object::Principal(p),
                None,
            )
            .unwrap();
        assembly.names.register(
                &assembly.roster,
                &mut assembly.tree,
                "named-league",
                Object::Coalition(coalition),
                None,
            )
            .unwrap();
        assembly.names.register(
                &assembly.roster,
                &mut assembly.tree,
                "named-league",
                Object::Coalition(coalition),
                None,
            )
            .unwrap();
        let other = organization.found(wait).unwrap();
        assert!(
            assembly.names.register(
                    &assembly.roster,
                    &mut assembly.tree,
                    "named-league",
                    Object::Coalition(other),
                    None
                )
                .is_err()
        );
        assert!(
            assembly.names.register(
                    &assembly.roster,
                    &mut assembly.tree,
                    "bad/name",
                    Object::Principal(p),
                    None
                )
                .is_err()
        );
        assembly.runtime.approve(crate::system::control::serve::resource::Approval {
                service,
                task: target,
                kind: "test".into(),
                name: "member".into(),
                permit: Permit::Identity(Selector::MemberOf(coalition)),
            })
            .unwrap();
        for i in 0..33 {
            assembly.runtime.approve(crate::system::control::serve::resource::Approval {
                    service,
                    task: target,
                    kind: "full".into(),
                    name: alloc::format!("f{i}"),
                    permit: Permit::Public,
                })
                .unwrap();
        }
        let source = mail::unseal_hole(env::Mark::of("hierarchy-stale-condition")).unwrap();
        assembly.publications.internal(
                &mut assembly.tree,
                protocol::common::path::Path::new("svc/fixtures/stale"),
                source,
                Permit::Identity(Selector::MemberOf(coalition)),
                me,
            )
            .unwrap();
    }
    standalone_mutations(assembly);
    assert!(matches!(
        operator.root().tile(
            protocol::common::path::Path::new("idt/principal/forged/ref"),
            wait
        ),
        Err(Fail::Unknown)
    ));
    sender_boundary(assembly);
    command(assembly, target, 1);
    let fake = mail::unseal_hole(protocol::system::control::publication::REF).unwrap();
    runtime::core::res::port::ship(
        &HolePie::from_token(fake),
        service,
        runtime::core::res::port::Access::STORE,
        runtime::core::res::port::Policy::NONE,
    )
    .unwrap();
    command(assembly, target, 5);
    let _ = mail::seal(fake);
    let _ = mail::release(fake);
    let service_road = assembly.runtime.runtime_road(service)
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
    protocol::debug::put(
        "hierarchy: runtime Exact permit allows another Task with same principal and denies different principal",
    );
    organization.expel(coalition, p, wait).unwrap();
    command(assembly, target, 2);
    let target_road = assembly.runtime.runtime_road(target)
        .unwrap();
    runtime::env::room::doom(target).unwrap();
    assert!(runtime::env::unit::join(target, wait).unwrap());
    assembly.progress().unwrap();
    assert!(matches!(
        operator.root().tile(&target_road, wait),
        Err(Fail::Unknown)
    ));
    command(assembly, target, 3);
    crate::system::control::serve::reap::sweep(&mut assembly.control, &assembly.roster);
    assembly.control.mint("system-child".into(), &assembly.images).unwrap();
    let failed_task = assembly.control.task("system-child").unwrap();
    let mut prepared = None;
    let mut fail_once = true;
    let machine = assembly.supplies.machine;
    let result = assembly
        .control
        .release("system-child".into(), service, &assembly.roster, &mut assembly.activation, &mut assembly.supplies, |control, activation| {
            crate::system::run::cycle::poll(control, &assembly.roster, &machine, activation, assembly.images.entry, &mut assembly.publications, &mut assembly.runtime, &mut assembly.names, &mut assembly.tree)?;
            if fail_once {
                prepared = assembly.runtime.runtime_road(failed_task);
                fail_once = false;
                Err("injected fixture preparation failure")
            } else {
                Ok(())
            }
        });
    assert!(result.is_err());
    assert!(
        assembly.runtime.runtime_road(failed_task)
            .is_none()
    );
    assert!(matches!(
        operator.root().tile(&prepared.unwrap(), wait),
        Err(Fail::Unknown)
    ));
    assert!(matches!(
        face(Grant::Resolve).call(Wire::Resolve(failed_task), wait),
        Ok(Reply::Binding(None))
    ));
    protocol::debug::put(
        "hierarchy: failed launch after identity/runtime preparation compensates bind and directories",
    );
    coalition
}

pub fn codecs() {
    use env::{PieToken, TaskId};
    use env::wire::Span as _;
    use protocol::system::operator::{EntryId, Permit, Tip, TipIn};
    use protocol::system::control::publication::{Frame, Object, Reply, Scope, Target};
    let a = TaskId::new(77);
    let b = PieToken::from_bytes(&81u64.to_le_bytes()).unwrap();
    let p = protocol::system::identity::PrincipalId::new(a, 19);
    for target in [
        Target::Service {
            scope: Scope::Driver,
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
        let frame = Frame::new(1, target.clone(), b, Permit::Public);
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
            scope: Scope::Fixture,
            group: "bad/name".into(),
            name: "x".into(),
        },
        b,
        Permit::Public,
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
    let mut bytes = [0; protocol::system::operator::TIP_LEN + 1];
    let tip = Tip::Plate {
        road: protocol::common::path::Path::new("svc/test/entry").to_path_buf(),
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
            road: protocol::common::path::Path::new("svc/test/entry").to_path_buf(),
            leaf: b,
            back: b,
        },
        Tip::Unplate {
            id: EntryId::new(79),
            back: b,
        },
        Tip::Empty {
            road: protocol::common::path::Path::new("uit/1/2").to_path_buf(),
            back: b,
        },
    ] {
        let n = tip.store(&mut bytes).unwrap();
        assert!(TipIn::fetch(&bytes[..n]).is_some());
        for end in 0..n {
            assert!(TipIn::fetch(&bytes[..end]).is_none());
        }
    }
    protocol::debug::put(
        "hierarchy: typed publication/ref/private mount codecs reject malformed and truncated frames",
    );
}

pub fn reference_lifetime() {
    use alloc::sync::Arc;
    use core::sync::atomic::{AtomicUsize, Ordering};
    use env::{Mark, Wait};
    use runtime::core::res::port::{self, Access, Policy};
    use runtime::env::mail::{self, HolePie};
    let me = runtime::env::unit::self_id();
    let source = mail::unseal_hole(Mark::of("forget-source")).unwrap();
    assert!(
        mail::forget(source).is_err(),
        "original resource ownership cannot be forgotten"
    );
    let borrowed = port::ship(
        &HolePie::from_token(source),
        me,
        Access::FETCH | Access::STORE,
        Policy::VEST,
    )
    .unwrap()
    .seed();
    let child = port::ship(
        &HolePie::from_token(borrowed),
        me,
        Access::FETCH | Access::STORE,
        Policy::VEST,
    )
    .unwrap()
    .seed();
    assert_eq!(mail::same(source, child), Ok(true));
    let other = mail::unseal_hole(Mark::of("forget-source")).unwrap();
    assert_eq!(mail::same(source, other), Ok(false));
    mail::forget(borrowed).unwrap();
    HolePie::from_token(child).push(b"ok", Wait::POLL).unwrap();
    let mut bytes = [0; 2];
    assert_eq!(
        HolePie::from_token(source)
            .pull(&mut bytes, Wait::POLL)
            .unwrap(),
        (2, me)
    );
    mail::revoke(me, child).unwrap();
    assert!(
        mail::reserve(child).is_err(),
        "reparented capability remains revocable upstream"
    );
    for round in 0..8 {
        let root = mail::unseal_hole(Mark::of("forget-race")).unwrap();
        let middle = port::ship(
            &HolePie::from_token(root),
            me,
            Access::FETCH | Access::STORE,
            Policy::VEST,
        )
        .unwrap()
        .seed();
        let stage = Arc::new(AtomicUsize::new(0));
        let progress = stage.clone();
        let root_id = me.get();
        let worker = runtime::core::task::join::closure(move || {
            let middle = protocol::communication::session::establish::claim(
                env::TaskId::new(root_id),
                Mark::of("forget-race"),
                Wait::AtMost(1000),
            )
            .unwrap();
            let me = runtime::env::unit::self_id();
            progress.store(1, Ordering::Release);
            while progress.load(Ordering::Acquire) < 2 {
                runtime::env::room::sleep(core::time::Duration::from_millis(1)).unwrap();
            }
            for _ in 0..32 {
                port::ship(
                    &HolePie::from_token(middle),
                    me,
                    Access::STORE,
                    Policy::NONE,
                )
                .unwrap();
            }
            progress.store(3, Ordering::Release);
            while progress.load(Ordering::Acquire) < 4 {
                runtime::env::room::sleep(core::time::Duration::from_millis(1)).unwrap();
            }
            assert!(
                !mail::pies().any(|p| p.mark == Mark::of("forget-race")),
                "revocation missed a concurrent descendant"
            );
        });
        let downstream = port::ship(
            &HolePie::from_token(middle),
            worker.id(),
            Access::FETCH | Access::STORE,
            Policy::VEST,
        )
        .unwrap()
        .seed();
        while stage.load(Ordering::Acquire) < 1 {
            runtime::env::room::sleep(core::time::Duration::from_millis(1)).unwrap();
        }
        stage.store(2, Ordering::Release);
        mail::forget(middle).unwrap();
        while stage.load(Ordering::Acquire) < 3 {
            runtime::env::room::sleep(core::time::Duration::from_millis(1)).unwrap();
        }
        if round % 2 == 0 {
            mail::revoke(worker.id(), downstream).unwrap();
        } else {
            mail::release(root).unwrap();
        }
        stage.store(4, Ordering::Release);
        worker.join();
        let _ = mail::seal(root);
        let _ = mail::release(root);
    }
    let _ = mail::seal(source);
    let _ = mail::release(source);
    let _ = mail::seal(other);
    let _ = mail::release(other);
    protocol::debug::put(
        "hierarchy: Forget preserves delivered capabilities, Same distinguishes objects, concurrent Accord keeps upstream revoke/release effective",
    );
}

fn sender_boundary(assembly: &mut crate::harness::probe::fixture::Fixture) {
    use alloc::sync::Arc;
    use core::sync::atomic::{AtomicBool, Ordering};
    use env::Wait;
    use protocol::system::operator::{Fail, Permit};
    use protocol::system::control::publication::{Client, ENTRY, Frame, Scope, Target};
    use runtime::core::res::port::{self, Access, Policy};
    use runtime::env::mail::{self, HolePie};
    let done = Arc::new(AtomicBool::new(false));
    let complete = done.clone();
    let control = runtime::env::unit::self_id().get();
    let protected = mail::unseal_hole(env::Mark::of("source-validation")).unwrap();
    let raw = protected.get();
    let caller = runtime::core::task::join::closure(move || {
        let control = env::TaskId::new(control);
        let entry =
            protocol::communication::session::establish::claim(control, ENTRY, Wait::AtMost(1000))
                .unwrap();
        let client = Client::direct(control, entry).unwrap();
        let target = Target::Service {
            scope: Scope::Fixture,
            group: "operator-fixture".into(),
            name: "entry".into(),
        };
        let source = mail::unseal_hole(env::Mark::of("publication-test")).unwrap();
        assert_eq!(
            client.publish(target.clone(), source, Permit::Public, Wait::AtMost(3000)),
            Err(Fail::Denied),
            "holding the entry does not install a trusted publisher record"
        );
        let other = env::PieToken::from_bytes(&(raw as u64).to_le_bytes()).unwrap();
        assert_eq!(
            client.call(
                Frame::new(1, target, other, Permit::Public),
                Wait::AtMost(3000)
            ),
            Err(Fail::Denied),
            "source reference must come from the actual sender"
        );
        complete.store(true, Ordering::Release);
    });
    let entry = assembly.images.entry;
    port::ship(
        &HolePie::from_token(entry),
        caller.id(),
        Access::STORE,
        Policy::NONE,
    )
    .unwrap();
    let until = runtime::env::chrono::clock() + 10_000_000_000;
    while !done.load(Ordering::Acquire) {
        assembly.progress().unwrap();
        assert!(
            runtime::env::chrono::clock() < until,
            "sender boundary timeout"
        );
        runtime::env::room::sleep(core::time::Duration::from_millis(1)).unwrap();
    }
    caller.join();
    assert!(
        mail::reserve(protected).is_ok(),
        "invalid request must not discard somebody else's reference"
    );
    let _ = mail::seal(protected);
    let _ = mail::release(protected);
    protocol::debug::put(
        "hierarchy: real kernel sender and transferred source checks reject unregistered caller and forged source",
    );
}

fn standalone_mutations(assembly: &mut crate::harness::probe::fixture::Fixture) {
    use alloc::sync::Arc;
    use core::sync::atomic::{AtomicBool, Ordering};
    use env::Wait;
    use protocol::communication::session::{Session, establish};
    use protocol::system::operator::{
        EntryId, Fail, Grant, Where,
        client::{self as operator, Face},
    };
    use runtime::core::res::port::{self, Access, Policy};
    use runtime::env::mail::HolePie;
    let control = runtime::env::unit::self_id();
    let host = assembly.tree.host().unwrap();
    for grant in [Grant::Part, Grant::Trim] {
        let done = Arc::new(AtomicBool::new(false));
        let complete = done.clone();
        let released = Arc::new(AtomicBool::new(false));
        let gate = released.clone();
        let root = control.get();
        let caller = runtime::core::task::join::closure(move || {
            while !gate.load(Ordering::Acquire) {
                runtime::env::room::sleep(core::time::Duration::from_millis(1)).unwrap();
            }
            let control = env::TaskId::new(root);
            let session =
                Session::open(control, operator::granted_berth(grant), Wait::AtMost(3000))
                    .unwrap_or_else(|_| panic!("standalone grant session"));
            let tree = Face::of(session);
            match grant {
                Grant::Part => assert_eq!(
                    tree.rein(grant)
                        .part(Where::Root, "idt".into(), Wait::AtMost(3000)),
                    Err(Fail::Denied)
                ),
                Grant::Trim => assert_eq!(
                    tree.rein(grant).trim(EntryId::new(0), Wait::AtMost(3000)),
                    Err(Fail::Denied)
                ),
                _ => unreachable!(),
            }
            let private = establish::claim(
                env::TaskId::new(host.get()),
                protocol::system::operator::TIP_MARK,
                Wait::AtMost(1000),
            )
            .unwrap();
            let tip = protocol::system::operator::Tip::Plate {
                road: protocol::common::path::Path::new("idt/principal/forged/ref").to_path_buf(),
                leaf: env::PieToken::NONE,
                permit: protocol::system::operator::Permit::Public,
                owner: control,
                replace: false,
                back: env::PieToken::NONE,
            };
            let mut bytes = [0; protocol::system::operator::TIP_LEN];
            let n = tip.store(&mut bytes).unwrap();
            HolePie::from_token(private)
                .push(&bytes[..n], Wait::AtMost(1000))
                .unwrap();
            complete.store(true, Ordering::Release);
        });
        assembly.roster
            .inherit(caller.id(), control)
            .unwrap();
        let private = establish::find(host, protocol::system::operator::TIP_MARK).unwrap();
        port::ship(
            &HolePie::from_token(private),
            caller.id(),
            Access::STORE,
            Policy::NONE,
        )
        .unwrap();
        released.store(true, Ordering::Release);
        let until = runtime::env::chrono::clock() + 10_000_000_000;
        while !done.load(Ordering::Acquire) {
            assembly
                .tree
                .connect(core::iter::once(caller.id()))
                .unwrap();
            assembly.progress().unwrap();
            assert!(
                runtime::env::chrono::clock() < until,
                "standalone raw mutation timeout"
            );
            runtime::env::room::sleep(core::time::Duration::from_millis(1)).unwrap();
        }
        let caller_id = caller.id();
        caller.join();
        assembly.roster.unbind(caller_id).unwrap();
    }
    protocol::debug::put(
        "hierarchy: standalone Part/Trim deny bound same-subject callers; private Plate still requires actual Control sender",
    );
}
