//! Trusted setup for the two Identity probes, not a product capability declaration.

use ::resource::port::{self, Access, Policy};
use env::Wait;
use ipc::session::establish;
use system_api::identity::Grant;

use crate::harness::probe::fixture::Fixture;
use crate::unit::{self, UnitFile};
use ::resource::raw::{Hole, inspect, reserve};
use env::pie;

pub(crate) fn supply_to(
    authority: Option<env::TaskId>,
    program: &UnitFile,
    task: env::TaskId,
) -> Result<(), &'static str> {
    if matches!(program.name(), "probe-rule" | "probe-rule-other") {
        let mark = env::Mark::of("probe-rule-verified");
        let owner = env::unit::self_id();
        let token = match establish::find(owner, mark) {
            Ok(token) => token,
            Err(establish::DiscoveryFail::Missing) => {
                pie::unseal_hole(mark).map_err(|_| "rule verification channel")?
            }
            Err(establish::DiscoveryFail::Ambiguous) => {
                return Err("rule verification channel ambiguous");
            }
        };
        let access = if program.name() == "probe-rule" {
            Access::FETCH
        } else {
            Access::STORE
        };
        port::ship(token, task, access, Policy::NONE).map_err(|_| "rule verification supply")?;
    }
    if program.name() == "system-dependent" {
        super::hierarchy::supply(task)?;
    }
    if program.name() != unit::probe_denied::PROBE_DENIED.name()
        && program.name() != unit::probe_coalition::PROBE_COALITION.name()
    {
        return Ok(());
    }
    let authority = authority.ok_or("identity fixture authority")?;
    for grant in [Grant::Bind, Grant::Unbind] {
        let token =
            establish::claim(authority, grant.mark(), Wait::POLL).map_err(
                |failure| match failure {
                    establish::DiscoveryFail::Missing => "identity fixture face",
                    establish::DiscoveryFail::Ambiguous => "identity fixture face ambiguous",
                },
            )?;
        if !matches!(reserve(token), Ok((_, owner, mark))
            if owner == authority && mark == grant.mark())
        {
            return Err("identity fixture source");
        }
        // Transfer an entry copy, not the installer's kernel sender identity.
        port::ship(token, task, Access::FETCH | Access::STORE, Policy::NONE)
            .map_err(|_| "identity fixture transfer")?;
    }
    Ok(())
}

/// A timed-out client may drop its send buffer while the kernel still holds the request.
pub fn timeout() {
    use alloc::sync::Arc;
    use core::sync::atomic::{AtomicBool, Ordering};
    use system_api::identity::Wire;
    use system_api::identity::limits::MAX_FRAME;
    use system_client::identity::CallError;
    use system_client::identity::Face;

    let owner = env::unit::self_id();
    let entry = pie::unseal_hole(Grant::Resolve.mark()).unwrap();
    let timed_out = Arc::new(AtomicBool::new(false));
    let release_reader = timed_out.clone();
    let raw_owner = owner.get();
    let reader = execution::unit::task::spawn(move || {
        let owner = env::TaskId::new(raw_owner);
        let entry = establish::claim(owner, Grant::Resolve.mark(), Wait::AtMost(1000)).unwrap();
        while !release_reader.load(Ordering::Acquire) {
            execution::room::park(core::time::Duration::from_millis(1)).unwrap();
        }
        let mut bytes = [0; MAX_FRAME];
        let (n, from) = Hole::from_raw(entry)
            .pull(&mut bytes, Wait::AtMost(1000))
            .unwrap();
        assert_eq!(from, owner);
        assert_eq!(
            Wire::take(&bytes[..n]).unwrap().0,
            Some(Wire::Resolve(owner))
        );
    });
    port::ship(entry, reader.id(), Access::FETCH, Policy::NONE).unwrap();
    let face = Face::direct(owner, Grant::Resolve, entry).unwrap();
    let result = face.call(Wire::Resolve(owner), Wait::AtMost(1));
    timed_out.store(true, Ordering::Release);
    assert_eq!(result, Err(CallError::Transport));
    reader.join();
    let _ = pie::seal(entry);
    let _ = pie::release(entry);
    programs::debug::put("identity-timeout: queued request decoded after client timeout");
}

/// Isolated supervisor fixture. Every observation goes over the real Identity IPC faces.
pub fn acceptance() {
    use crate::system::app::{bootstrap, scene};
    use system_client::identity::TaskQuery;

    super::hierarchy::codecs();
    super::hierarchy::reference_lifetime();
    let boot = bootstrap::take().expect("identity: bootstrap");
    let list = scene::programs(&boot.catalog).expect("identity: scene");
    let mut assembly = Fixture::new(boot).ok().expect("identity: assembly");
    revision(&mut assembly);
    for program in list {
        if program.name() == "system-child" {
            assembly
                .resources
                .write::<crate::system::control::unit::Control>()
                .unwrap()
                .enlist(program)
                .expect("identity: runtime declaration");
            continue;
        }
        assembly
            .assemble(program)
            .expect("identity: install static unit");
    }
    let query = |authority| {
        let face = |grant: Grant| {
            establish::find(authority, grant.mark()).expect("identity: face missing")
        };
        TaskQuery::direct(
            authority,
            face(Grant::Resolve),
            face(Grant::Matches),
            face(Grant::Same),
        )
        .expect("identity: face source")
    };
    for name in ["operator", "identity"] {
        assert!(
            assembly
                .action(name, crate::system::control::lifecycle::Action::Mint)
                .is_err()
        );
        assert!(
            assembly
                .action(name, crate::system::control::lifecycle::Action::Ruin)
                .is_err()
        );
        assert_eq!(
            assembly
                .resources
                .read::<crate::system::control::unit::Control>()
                .unwrap()
                .state(name.into())
                .unwrap(),
            crate::system::control::unit::table::State::Ready
        );
    }
    let old_authority = assembly
        .resources
        .read::<crate::system::control::identity::Roster>()
        .unwrap()
        .authority()
        .unwrap();
    let old_query = query(old_authority);
    let me = env::unit::self_id();
    let host = assembly
        .resources
        .read::<crate::system::operator::management::Tree>()
        .unwrap()
        .host()
        .unwrap();
    let link = establish::endpoint(host, system_api::operator::LINK_MARK, Wait::POLL)
        .expect("identity: operator request");
    let (talk, ask) =
        establish::give_at(host, system_api::operator::ASK_MARK).expect("identity: operator ask");
    let tip = establish::find(host, system_api::operator::TIP_MARK)
        .unwrap_or_else(|_| panic!("identity: trusted operator tip missing or ambiguous"));
    let mut record = [0u8; system_api::operator::TIP_LEN];
    let explicit_reply = link.seed();
    let n = system_api::operator::Tip::Guest {
        who: me,
        reply: explicit_reply,
        ask,
    }
    .store(&mut record)
    .unwrap();
    Hole::from_raw(tip)
        .push(&record[..n], Wait::AtMost(1000))
        .expect("identity: trusted guest registration");
    let mut ack = [0; 1];
    let (len, from) = Hole::from_raw(link.rx())
        .pull(&mut ack, Wait::AtMost(1000))
        .expect("identity: Operator admission acknowledgement");
    assert_eq!((len, from, ack[0]), (1, host, system_api::operator::OK));
    let operator = system_client::operator::Face::of(
        unsafe { ipc::session::Session::from_raw(link, talk, host) }
            .unwrap_or_else(|_| panic!("identity: owned Operator session")),
    );
    super::loader::acceptance(&mut assembly, &operator);
    super::account::acceptance(&mut assembly, &operator);
    let protected = || {
        operator
            .tile(
                system_api::operator::path::Path::new("/svc/sys/control/mint"),
                Wait::AtMost(1000),
            )
            .expect("identity: protected tile")
    };
    assert!(
        protected().token(Wait::AtMost(1000)).is_ok(),
        "identity: trusted operation"
    );
    let public = operator
        .tile(
            system_api::operator::path::Path::new("/svc/sys/control/state"),
            Wait::AtMost(1000),
        )
        .expect("identity: public tile");
    assert!(public.token(Wait::AtMost(1000)).is_ok());
    let old = old_query.resolve(me, Wait::AtMost(1000)).unwrap().unwrap();
    let old_dependent = assembly
        .resources
        .read::<crate::system::control::unit::Control>()
        .unwrap()
        .task("system-dependent")
        .unwrap();
    let old_hub = assembly
        .resources
        .read::<crate::system::control::unit::Control>()
        .unwrap()
        .task("hub")
        .unwrap();
    let old_devices = old_query
        .resolve(old_hub, Wait::AtMost(1000))
        .unwrap()
        .unwrap()
        .current;
    assert!(
        !old_devices.coalitions.is_empty(),
        "identity: Hub selections not activated"
    );
    let coalition = old_devices.coalitions.iter().next().unwrap();
    activation_boundary(&assembly, old_hub, coalition);
    let before = old_query
        .resolve(old_dependent, Wait::AtMost(1000))
        .unwrap();
    assert!(
        assembly
            .resources
            .read::<crate::system::control::identity::Roster>()
            .unwrap()
            .activate(old_dependent, coalition)
            .is_err(),
        "identity: qualification was not checked"
    );
    assert_eq!(
        old_query
            .resolve(old_dependent, Wait::AtMost(1000))
            .unwrap(),
        before
    );
    for (name, road) in [
        ("router", "/svc/drv/router"),
        ("rtc", "/svc/drv/rtc"),
        ("uart", "/svc/drv/uart/rx"),
    ] {
        ready_driver(
            &operator,
            &old_query,
            assembly
                .resources
                .read::<crate::system::control::unit::Control>()
                .unwrap()
                .task(name)
                .unwrap(),
            road,
        );
    }
    use crate::system::control::lifecycle::Action;
    assert!(
        assembly.action("absent-unit", Action::Ruin).is_err(),
        "unknown Ruin must fail without terminating System"
    );
    assembly
        .progress()
        .expect("System remains usable after unknown request");

    assembly
        .action("system-child", Action::Mint)
        .expect("identity: scheduled mint");
    let child = assembly
        .action(
            "system-child",
            Action::Embark {
                parent: Some(old_dependent),
            },
        )
        .expect("identity: scheduled inheritance")
        .unwrap();
    let parent_binding = old_query
        .resolve(old_dependent, Wait::AtMost(1000))
        .unwrap()
        .unwrap();
    let child_binding = old_query
        .resolve(child, Wait::AtMost(1000))
        .unwrap()
        .unwrap();
    assert_eq!(child_binding.origin, parent_binding.current);
    for _ in 0..3 {
        assembly
            .action("system-child", Action::Debark)
            .expect("identity: scheduled debark");
        assert_eq!(
            assembly
                .resources
                .read::<crate::system::control::unit::Control>()
                .unwrap()
                .state("system-child".into())
                .unwrap(),
            crate::system::control::unit::table::State::Debarked
        );
        assert_eq!(
            old_query
                .resolve(child, Wait::AtMost(1000))
                .unwrap()
                .unwrap(),
            child_binding
        );
        let resumed = assembly
            .action(
                "system-child",
                Action::Embark {
                    parent: Some(old_dependent),
                },
            )
            .expect("identity: scheduled resume")
            .unwrap();
        assert_eq!(resumed, child);
        assert_eq!(
            old_query
                .resolve(child, Wait::AtMost(1000))
                .unwrap()
                .unwrap(),
            child_binding
        );
    }

    programs::debug::put("identity: three Debark/Embark rounds preserve task and binding");
    assert_eq!(old.current.principal.authority, old_authority);
    let dynamic = super::hierarchy::exercise(
        &mut assembly,
        &operator,
        old_authority,
        old_dependent,
        child,
    );
    let _ = dynamic;
    programs::debug::put("system: identity, device and publication acceptance passed");
    assembly.settle();
    assert!(
        assembly.supervise().is_ok(),
        "system: normal team shutdown failed"
    );
}

fn revision(assembly: &mut Fixture) {
    use crate::system::{
        control::identity::Roster,
        identity::revision::{Changed, Epoch},
    };
    use core::sync::atomic::Ordering;
    use system_api::identity::PrincipalId;
    use system_api::identity::Reply;
    use system_api::identity::Wire;
    use system_client::identity::CallError;
    use system_client::identity::Face;

    assembly.progress().expect("identity: initial maintenance");
    let authority = assembly
        .resources
        .read::<Roster>()
        .unwrap()
        .authority()
        .unwrap();
    let epoch = assembly.resources.read::<Epoch>().unwrap().clone();
    let changed = assembly.resources.read::<Changed>().unwrap();
    assert!(
        !changed.0.wait(Wait::POLL).unwrap(),
        "identity: initial changes not consumed"
    );
    let before = epoch.0.load(Ordering::Acquire);
    let query = Face::direct(
        authority,
        Grant::Resolve,
        establish::find(authority, Grant::Resolve.mark()).unwrap(),
    )
    .unwrap();
    assert!(matches!(
        query.call(Wire::Resolve(env::unit::self_id()), Wait::AtMost(1000)),
        Ok(Reply::Binding(Some(_)))
    ));
    assert_eq!(
        epoch.0.load(Ordering::Acquire),
        before,
        "identity: query changed revision"
    );
    assert!(
        !changed.0.wait(Wait::POLL).unwrap(),
        "identity: query woke maintenance"
    );
    let derive = Face::direct(
        authority,
        Grant::Derive,
        establish::find(authority, Grant::Derive.mark()).unwrap(),
    )
    .unwrap();
    assert!(matches!(
        derive.call(
            Wire::Derive(PrincipalId::new(authority, u64::MAX)),
            Wait::AtMost(1000)
        ),
        Err(CallError::Service(_))
    ));
    assert_eq!(
        epoch.0.load(Ordering::Acquire),
        before,
        "identity: denial changed revision"
    );
    assert!(
        !changed.0.wait(Wait::POLL).unwrap(),
        "identity: denial woke maintenance"
    );
    assert!(matches!(
        derive.call(
            Wire::Derive(PrincipalId::root(authority)),
            Wait::AtMost(1000)
        ),
        Ok(Reply::Principal(_))
    ));
    assert_eq!(
        epoch.0.load(Ordering::Acquire),
        before + 1,
        "identity: mutation revision missing"
    );
    assert!(
        changed.0.wait(Wait::POLL).unwrap(),
        "identity: mutation did not wake maintenance"
    );
    drop(changed);
    assembly.progress().expect("identity: changed maintenance");
    assert_eq!(
        crate::system::publication::observed_revision(&assembly.resources).unwrap(),
        before + 1
    );
    assert!(
        !assembly
            .resources
            .read::<Changed>()
            .unwrap()
            .0
            .wait(Wait::POLL)
            .unwrap(),
        "identity: mutation notification not consumed"
    );
    programs::debug::put("identity: only successful mutations wake maintenance");
}

fn activation_boundary(
    assembly: &Fixture,
    hub: env::TaskId,
    coalition: system_api::identity::CoalitionId,
) {
    use alloc::sync::Arc;
    use core::sync::atomic::{AtomicBool, Ordering};
    use hub_api::activation;

    let owner = env::unit::self_id();
    let entry = establish::find(owner, activation::ENTRY).unwrap();
    let done = Arc::new(AtomicBool::new(false));
    let finished = done.clone();
    let (owner, hub, authority, slot) = (
        owner.get(),
        hub.get(),
        coalition.authority.get(),
        coalition.slot,
    );
    let caller = execution::unit::task::spawn(move || {
        let owner = env::TaskId::new(owner);
        let hub = env::TaskId::new(hub);
        let coalition = system_api::identity::CoalitionId::new(env::TaskId::new(authority), slot);
        let entry = establish::claim(owner, activation::ENTRY, Wait::AtMost(1000)).unwrap();
        assert!(
            port::ship(entry, hub, Access::STORE, Policy::NONE).is_err(),
            "activation copy unexpectedly transferable"
        );
        assert!(
            hub_client::activate(hub, &[coalition]).is_err(),
            "activation accepted a non-Hub kernel sender"
        );
        finished.store(true, Ordering::Release);
    });
    port::ship(entry, caller.id(), Access::STORE, Policy::NONE).unwrap();
    let until = env::chrono::clock() + 5_000_000_000;
    while !done.load(Ordering::Acquire) {
        crate::system::launch::activation::maintain(
            assembly.resources.read().unwrap(),
            assembly.resources.read().unwrap(),
            assembly.resources.read().unwrap(),
        )
        .unwrap();
        assert!(
            env::chrono::clock() < until,
            "activation boundary never answered"
        );
        execution::room::park(core::time::Duration::from_millis(1)).unwrap();
    }
    caller.join();
}

fn ready_driver(
    operator: &system_client::operator::Face,
    query: &system_client::identity::TaskQuery,
    task: env::TaskId,
    road: &'static str,
) {
    let until = env::chrono::clock() + 5_000_000_000;
    loop {
        if let Ok(entry) = operator
            .tile(
                system_api::operator::path::Path::new(road),
                Wait::AtMost(1000),
            )
            .and_then(|tile| tile.token(Wait::AtMost(1000)))
        {
            if matches!(inspect(entry), Ok((_, owner, _)) if owner == task) {
                let binding = query.resolve(task, Wait::AtMost(1000)).unwrap().unwrap();
                assert!(
                    !binding.current.coalitions.is_empty(),
                    "identity: driver inactive"
                );
                assert_eq!(binding.current.principal.authority, query.authority());
                return;
            }
        }
        assert!(
            env::chrono::clock() < until,
            "identity: driver never republished"
        );
        execution::room::park(core::time::Duration::from_millis(1)).unwrap();
    }
}
