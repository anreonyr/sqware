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

fn execute(
    assembly: &mut crate::harness::probe::fixture::Fixture,
    task: env::TaskId,
    request: (u8, Option<env::PieToken>),
) {
    let (code, reference) = request;
    use env::Wait;
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
    use env::{Access, Mark, Wait};
    use ipc::session::establish;
    use system_api::{identity, operator as api};
    use system_client::{
        identity::{Face, Installer, Organization},
        operator::Capability,
    };
    let wait = Wait::AtMost(3000);
    let entry = |g: identity::Grant| establish::find(authority, g.mark()).unwrap();
    let face = |g| Face::direct(authority, g, entry(g)).unwrap();
    let me = env::unit::self_id();
    let identity::Reply::Binding(Some(saved)) = face(identity::Grant::Resolve)
        .call(identity::Wire::Resolve(me), wait)
        .unwrap()
    else {
        panic!("system binding")
    };
    let identity::Reply::Binding(Some(binding)) = face(identity::Grant::Resolve)
        .call(identity::Wire::Resolve(service), wait)
        .unwrap()
    else {
        panic!("service binding")
    };
    let p = binding.current.principal;
    command(assembly, target, 1);
    let organization = Organization::direct(
        authority,
        entry(identity::Grant::Found),
        entry(identity::Grant::Admit),
        entry(identity::Grant::Expel),
    )
    .unwrap();
    let coalition = organization.found(wait).unwrap();
    organization.admit(coalition, p, wait).unwrap();
    let memberships = identity::CoalitionSet::new(&[coalition]).unwrap();
    face(identity::Grant::Activate)
        .call(identity::Wire::Activate(service, memberships), wait)
        .unwrap();
    face(identity::Grant::Activate)
        .call(identity::Wire::Activate(target, memberships), wait)
        .unwrap();

    let start = assembly.resources.read::<crate::system::app::operator::Layout>().unwrap()
        .as_ref().unwrap().reserve_operations(202).unwrap().0;
    let root = assembly
        .resources
        .read::<crate::system::operator::management::Tree>()
        .unwrap()
        .root_entry()
        .unwrap();
    let capability = Capability::direct_at(root.entry.operator, root.entrance, root.entry).unwrap();
    let pane = capability
        .part(
            root.entry,
            "hierarchy-fixture",
            api::OperationId(start),
            wait,
        )
        .unwrap();
    let replay = capability
        .part(
            root.entry,
            "hierarchy-fixture",
            api::OperationId(start),
            wait,
        )
        .unwrap();
    assert_eq!(
        pane.entry, replay.entry,
        "PART replay must retain the original Entry"
    );
    let source = pie::unseal(env::UnsealArgs::hole(Mark::of("hierarchy-resource"))).unwrap();
    let view = api::ResourceView::new(Access::FETCH_STORE).unwrap();
    let member = capability
        .bind(
            pane.entry,
            "member",
            source,
            view,
            api::Permit::Identity(identity::Selector::MemberOf(coalition)),
            api::OperationId(start + 1),
            wait,
        )
        .unwrap()
        .binding;
    let exact = capability
        .bind(
            pane.entry,
            "exact",
            source,
            view,
            api::Permit::Identity(identity::Selector::Exact(p)),
            api::OperationId(start + 2),
            wait,
        )
        .unwrap()
        .binding;
    assert_eq!(
        capability
            .bind(
                pane.entry,
                "exact",
                source,
                view,
                api::Permit::Identity(identity::Selector::Exact(p)),
                api::OperationId(start + 2),
                wait
            )
            .unwrap()
            .binding,
        exact
    );
    assert_eq!(
        capability
            .read(
                exact.entry,
                api::Permit::Identity(identity::Selector::Exact(p)),
                wait
            )
            .err(),
        Some(api::Fail::Denied)
    );
    let installer = Installer::direct(
        authority,
        entry(identity::Grant::Bind),
        entry(identity::Grant::Unbind),
        entry(identity::Grant::Derive),
    )
    .unwrap();
    installer
        .bind(
            me,
            identity::Install::Authorized(identity::Subject::new(p, &[coalition]).unwrap()),
            wait,
        )
        .unwrap();
    let acquired = capability
        .read(
            exact.entry,
            api::Permit::Identity(identity::Selector::Exact(p)),
            wait,
        )
        .unwrap();
    assert_eq!(reserve(acquired.resource).unwrap().1, me);
    let read = capability
        .read(
            member.entry,
            api::Permit::Identity(identity::Selector::MemberOf(coalition)),
            wait,
        )
        .unwrap();
    assert_eq!(
        read.binding, member,
        "direct reference must retain actual Binding revision"
    );

    let trusted = Installer::direct_with_install(
        authority,
        entry(identity::Grant::Install),
        entry(identity::Grant::Derive),
    )
    .unwrap();
    let alias = identity::IdentityName::new("named-subject").unwrap();
    let object = identity::IdentityObject::Principal(p);
    let reference = identity::OperatorEntryRef {
        operator: exact.entry.operator,
        root: matches!(exact.entry.target, api::EntryTarget::Root),
        id: match exact.entry.target {
            api::EntryTarget::Root => 0,
            api::EntryTarget::Node(id) => id.get() as u64,
        },
        revision: exact.revision,
    };
    trusted.associate(object, alias, reference, wait).unwrap();
    trusted.associate(object, alias, reference, wait).unwrap();
    assert_eq!(
        face(identity::Grant::Reference)
            .call(identity::Wire::Reference(object, alias), wait)
            .unwrap(),
        identity::Reply::Reference(Some(reference))
    );
    let wrong = identity::IdentityObject::Principal(identity::PrincipalId::new(
        env::TaskId::new(authority.get() + 10000),
        p.slot,
    ));
    assert!(
        trusted.associate(wrong, alias, reference, wait).is_err(),
        "foreign Identity authority must be rejected"
    );
    assert!(identity::IdentityName::new("bad/name").is_none());
    let fake = pie::unseal(env::UnsealArgs::hole(identity::Grant::Resolve.mark())).unwrap();
    assert!(
        Face::direct(authority, identity::Grant::Resolve, fake).is_err(),
        "a matching Mark does not establish Identity provenance"
    );
    pie::release(fake, env::ReleaseMode::Revoke).unwrap();

    organization.expel(coalition, p, wait).unwrap();
    assert_eq!(
        capability
            .read(
                member.entry,
                api::Permit::Identity(identity::Selector::MemberOf(coalition)),
                wait
            )
            .err(),
        Some(api::Fail::Denied)
    );
    capability.unbind(member, wait).unwrap();
    assert!(
        capability
            .read(member.entry, api::Permit::Public, wait)
            .is_err()
    );
    Hole::from_raw(read.resource).push(b"kept", wait).unwrap();
    let mut bytes = [0; 4];
    assert_eq!(
        Hole::from_raw(source).pull(&mut bytes, wait).unwrap(),
        (4, me)
    );
    assert_eq!(
        &bytes, b"kept",
        "namespace unbind must preserve delivered copies"
    );
    pie::seal(source).unwrap();
    assert!(
        reserve(read.resource).is_err(),
        "provider close must invalidate copies"
    );
    installer
        .bind(me, identity::Install::Authorized(saved.current), wait)
        .unwrap();
    capability.unbind(exact, wait).unwrap();
    let _ = pie::release(source, env::ReleaseMode::Revoke);
    let _ = pie::release(acquired.resource, env::ReleaseMode::Keep);
    let _ = pie::release(read.resource, env::ReleaseMode::Keep);

    let source = pie::unseal(env::UnsealArgs::hole(Mark::of("hierarchy-capacity"))).unwrap();
    let mut bindings = alloc::vec::Vec::new();
    for index in 0..api::frame::PANE_CAP {
        bindings.push(
            capability
                .bind(
                    pane.entry,
                    &alloc::format!("f{index}"),
                    source,
                    view,
                    api::Permit::Public,
                    api::OperationId(start + 100 + index as u64),
                    wait,
                )
                .unwrap()
                .binding,
        );
    }
    let retry_id = api::OperationId(start + 200);
    assert!(
        capability
            .bind(
                pane.entry,
                "overflow",
                source,
                view,
                api::Permit::Public,
                retry_id,
                wait
            )
            .is_err()
    );
    capability.unbind(bindings.remove(0), wait).unwrap();
    assert!(
        capability
            .bind(
                pane.entry,
                "overflow",
                source,
                view,
                api::Permit::Public,
                retry_id,
                wait
            )
            .is_err(),
        "failed operation remains terminal"
    );
    bindings.push(
        capability
            .bind(
                pane.entry,
                "overflow",
                source,
                view,
                api::Permit::Public,
                api::OperationId(start + 201),
                wait,
            )
            .unwrap()
            .binding,
    );
    for binding in bindings {
        capability.unbind(binding, wait).unwrap();
    }
    capability.unpart(pane.entry, wait).unwrap();
    assert!(
        capability
            .find(root.entry, "hierarchy-fixture", api::Permit::Public, wait)
            .is_err()
    );
    pie::release(source, env::ReleaseMode::Revoke).unwrap();
    standalone_mutations(assembly);

    assembly
        .action(
            "system-child",
            crate::system::control::lifecycle::Action::Ruin,
        )
        .unwrap();
    assert!(env::unit::join_task(target, wait).unwrap_or(true));
    assert!(
        !assembly
            .resources
            .read::<crate::system::assembly::namespace::NamespaceInstallations>()
            .unwrap()
            .prepared(target)
    );
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
    assert!(
        assembly
            .resources
            .read::<super::fixture::Fault>()
            .unwrap()
            .prepared,
        "fault must follow completed namespace preparation"
    );
    assert!(
        !assembly
            .resources
            .read::<crate::system::assembly::namespace::NamespaceInstallations>()
            .unwrap()
            .prepared(failed_task)
    );
    assert!(matches!(
        face(identity::Grant::Resolve).call(identity::Wire::Resolve(failed_task), wait),
        Ok(identity::Reply::Binding(None))
    ));
    programs::debug::put(
        "hierarchy: direct Entry/identity references, Exact/MemberOf permits, replay, capacity, Keep/close and preparation rollback passed",
    );
    let _ = operator;
    coalition
}

pub fn codecs() {
    use env::wire::Span as _;
    use system_api::{identity, operator as api};
    use wire::Message;
    let request = api::entry::Request {
        op: api::entry::BIND,
        anchor: api::EntryId::new(0),
        root: 0,
        id: 1,
        parent: 0,
        holder: env::TaskId::new(0),
        ops: 0,
        permit: api::Permit::Public,
        permit_restricted: 0,
        source: env::TaskId::new(0),
        source_mark: 0,
        resource: env::PieToken::NONE,
        resource_id: 0,
        view: env::Permission::STORE.bits(),
        expected: 0,
        name: "entry".into(),
        scope: "".into(),
        back: env::PieToken::NONE,
    };
    let mut bytes = [0; api::entry::Request::LEN + 1];
    let n = request.store_at(&mut bytes, 0).unwrap();
    assert_eq!(api::entry::Request::fetch(&bytes[..n]), Some(request));
    assert!(api::entry::Request::fetch(&bytes[..n + 1]).is_none());
    for end in 0..n {
        assert!(api::entry::Request::fetch(&bytes[..end]).is_none());
    }
    assert!(identity::IdentityName::new("bad/name").is_none());
    let node = api::Entry {
        operator: env::TaskId::new(77),
        target: api::EntryTarget::Node(api::EntryId::new(0)),
    };
    assert_ne!(node.target, api::EntryTarget::Root);
    programs::debug::put(
        "hierarchy: direct operator frames reject trailing/truncated payloads and retain Node(0)",
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

fn standalone_mutations(assembly: &mut crate::harness::probe::fixture::Fixture) {
    use ::resource::port::{self, Access, Policy};
    use alloc::sync::Arc;
    use core::sync::atomic::{AtomicBool, Ordering};
    use env::Wait;
    use ipc::session::{Session, establish};
    use system_api::operator::Fail;
    use system_client::operator;
    use system_client::operator::Face;
    let control = env::unit::self_id();
    let host = assembly
        .resources
        .read::<crate::system::operator::management::Tree>()
        .unwrap()
        .host()
        .unwrap();
    for _ in 0..2 {
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
            let forged = env::pie::unseal(env::UnsealArgs::hole(
                system_api::operator::entry::AUTH_ENTRY,
            ))
            .unwrap();
            assert!(matches!(
                operator::Capability::direct(tree.host(), forged),
                Err(Fail::Denied)
            ));
            let _ = env::pie::release(forged, env::ReleaseMode::Revoke);
            let private = establish::claim(
                env::TaskId::new(host.get()),
                system_api::operator::TIP_MARK,
                Wait::AtMost(1000),
            )
            .unwrap();
            Hole::from_raw(private)
                .push(&[1u8; 1], Wait::AtMost(1000))
                .unwrap();
            complete.store(true, Ordering::Release);
        });
        assembly
            .resources
            .write::<crate::system::control::identity::Roster>()
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
            .write::<crate::system::control::identity::Roster>()
            .unwrap()
            .unbind(caller_id)
            .unwrap();
    }
    programs::debug::put(
        "hierarchy: standalone Part/Trim deny bound same-subject callers; private Plate still requires actual Control sender",
    );
}
