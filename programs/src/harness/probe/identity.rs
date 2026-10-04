//! Trusted setup for the two Identity probes, not a product capability declaration.

use env::Wait;
use protocol::communication::session::establish;
use protocol::system::identity::Grant;
use runtime::core::res::port::{self, Access, Policy};

use crate::harness::probe::fixture::Fixture;
use crate::unit::{self, UnitFile};
use env::pie;
use runtime::core::res::pie::{HolePie, inspect, reserve};
use runtime::core::adapt;

pub(crate) fn supply_to(
    authority: Option<env::TaskId>,
    program: &UnitFile,
    task: env::TaskId,
) -> Result<(), &'static str> {
    if matches!(program.name(), "probe-rule" | "probe-rule-other") {
        let mark = env::Mark::of("probe-rule-verified");
        let owner = env::unit::self_id();
        let token = establish::find(owner, mark)
            .or_else(|| pie::unseal_hole(mark).ok())
            .ok_or("rule verification channel")?;
        let access = if program.name() == "probe-rule" {
            Access::FETCH
        } else {
            Access::STORE
        };
        port::ship(&HolePie::from_token(token), task, access, Policy::NONE)
            .map_err(|_| "rule verification supply")?;
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
            establish::claim(authority, grant.mark(), Wait::POLL).ok_or("identity fixture face")?;
        if !matches!(reserve(token), Ok((_, owner, mark))
            if owner == authority && mark == grant.mark())
        {
            return Err("identity fixture source");
        }
        // Transfer an entry copy, not the installer's kernel sender identity.
        port::ship(
            &HolePie::from_token(token),
            task,
            Access::FETCH | Access::STORE,
            Policy::NONE,
        )
        .map_err(|_| "identity fixture transfer")?;
    }
    Ok(())
}

/// A timed-out client may drop its send buffer while the kernel still holds the request.
pub fn timeout() {
    use alloc::sync::Arc;
    use core::sync::atomic::{AtomicBool, Ordering};
    use protocol::system::identity::{
        Wire,
        client::{CallError, Face},
        limits::MAX_FRAME,
    };

    let owner = env::unit::self_id();
    let entry = pie::unseal_hole(Grant::Resolve.mark()).unwrap();
    let timed_out = Arc::new(AtomicBool::new(false));
    let release_reader = timed_out.clone();
    let raw_owner = owner.get();
    let reader = runtime::core::task::join::closure(move || {
        let owner = env::TaskId::new(raw_owner);
        let entry = establish::claim(owner, Grant::Resolve.mark(), Wait::AtMost(1000)).unwrap();
        while !release_reader.load(Ordering::Acquire) {
            adapt::sleep(core::time::Duration::from_millis(1)).unwrap();
        }
        let mut bytes = [0; MAX_FRAME];
        let (n, from) = HolePie::from_token(entry)
            .pull(&mut bytes, Wait::AtMost(1000))
            .unwrap();
        assert_eq!(from, owner);
        assert_eq!(
            Wire::take(&bytes[..n]).unwrap().0,
            Some(Wire::Resolve(owner))
        );
    });
    port::ship(
        &HolePie::from_token(entry),
        reader.id(),
        Access::FETCH,
        Policy::NONE,
    )
    .unwrap();
    let face = Face::direct(owner, Grant::Resolve, entry).unwrap();
    let result = face.call(Wire::Resolve(owner), Wait::AtMost(1));
    timed_out.store(true, Ordering::Release);
    assert_eq!(result, Err(CallError::Transport));
    reader.join();
    let _ = pie::seal(entry);
    let _ = pie::release(entry);
    protocol::debug::put("identity-timeout: queued request decoded after client timeout");
}

/// Isolated supervisor fixture. Every observation goes over the real Identity IPC faces.
pub fn acceptance() {
    use crate::system::run::{bootstrap, scene};
    use protocol::system::identity::client::TaskQuery;

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
                .write::<crate::system::control::serve::unit::Control>()
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
                .action(name, crate::system::control::serve::lifecycle::Action::Mint)
                .is_err()
        );
        assert!(
            assembly
                .action(name, crate::system::control::serve::lifecycle::Action::Ruin)
                .is_err()
        );
        assert_eq!(
            assembly
                .resources
                .read::<crate::system::control::serve::unit::Control>()
                .unwrap()
                .state(name.into())
                .unwrap(),
            crate::system::control::core::unit::State::Ready
        );
    }
    let old_authority = assembly
        .resources
        .read::<crate::system::identity::serve::install::Roster>()
        .unwrap()
        .authority()
        .unwrap();
    let old_query = query(old_authority);
    let me = env::unit::self_id();
    let host = assembly
        .resources
        .read::<crate::system::operator::serve::install::Tree>()
        .unwrap()
        .host()
        .unwrap();
    let link = establish::endpoint(
        host,
        env::Mark::of(protocol::system::operator::LINK),
        Wait::POLL,
    )
    .expect("identity: operator request");
    let talk = establish::give(host, protocol::system::operator::ASK_MARK)
        .expect("identity: operator ask");
    let tip = establish::find(host, protocol::system::operator::TIP_MARK)
        .expect("identity: trusted operator tip");
    let mut record = [0u8; protocol::system::operator::TIP_LEN];
    let n = protocol::system::operator::Tip::Guest(me)
        .store(&mut record)
        .unwrap();
    HolePie::from_token(tip)
        .push(&record[..n], Wait::AtMost(1000))
        .expect("identity: trusted guest registration");
    let operator =
        protocol::system::operator::client::Face::of(protocol::communication::session::Session {
            link,
            talk,
            host,
        });
    let protected = || {
        operator
            .tile(
                protocol::common::path::Path::new("/svc/sys/control/mint"),
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
            protocol::common::path::Path::new("/svc/sys/control/state"),
            Wait::AtMost(1000),
        )
        .expect("identity: public tile");
    assert!(public.token(Wait::AtMost(1000)).is_ok());
    let old = old_query.resolve(me, Wait::AtMost(1000)).unwrap().unwrap();
    let old_dependent = assembly
        .resources
        .read::<crate::system::control::serve::unit::Control>()
        .unwrap()
        .task("system-dependent")
        .unwrap();
    let old_hub = assembly
        .resources
        .read::<crate::system::control::serve::unit::Control>()
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
            .read::<crate::system::identity::serve::install::Roster>()
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
                .read::<crate::system::control::serve::unit::Control>()
                .unwrap()
                .task(name)
                .unwrap(),
            road,
        );
    }
    use crate::system::control::serve::lifecycle::Action;
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
                .read::<crate::system::control::serve::unit::Control>()
                .unwrap()
                .state("system-child".into())
                .unwrap(),
            crate::system::control::core::unit::State::Debarked
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

    protocol::debug::put("identity: three Debark/Embark rounds preserve task and binding");
    assert_eq!(old.current.principal.authority, old_authority);
    let dynamic = super::hierarchy::exercise(
        &mut assembly,
        &operator,
        old_authority,
        old_dependent,
        child,
    );
    let _ = dynamic;
    protocol::debug::put("system: identity, device and publication acceptance passed");
    assembly
        .resources
        .write::<crate::system::control::serve::frame::Flow>()
        .unwrap()
        .settling = true;
    assert!(
        assembly.supervise().is_ok(),
        "system: normal team shutdown failed"
    );
}

fn revision(assembly: &mut Fixture) {
    use crate::system::identity::serve::{
        install::Roster,
        revision::{Changed, Epoch},
    };
    use core::sync::atomic::Ordering;
    use protocol::system::identity::{
        PrincipalId, Reply, Wire,
        client::{CallError, Face},
    };

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
        query.call(
            Wire::Resolve(env::unit::self_id()),
            Wait::AtMost(1000)
        ),
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
        assembly
            .resources
            .read::<crate::system::identity::serve::names::Registrations>()
            .unwrap()
            .seen,
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
    protocol::debug::put("identity: only successful mutations wake maintenance");
}

fn activation_boundary(
    assembly: &Fixture,
    hub: env::TaskId,
    coalition: protocol::system::identity::CoalitionId,
) {
    use alloc::sync::Arc;
    use core::sync::atomic::{AtomicBool, Ordering};
    use protocol::service::hub::activation;

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
    let caller = runtime::core::task::join::closure(move || {
        let owner = env::TaskId::new(owner);
        let hub = env::TaskId::new(hub);
        let coalition =
            protocol::system::identity::CoalitionId::new(env::TaskId::new(authority), slot);
        let entry = establish::claim(owner, activation::ENTRY, Wait::AtMost(1000)).unwrap();
        assert!(
            port::ship(
                &HolePie::from_token(entry),
                hub,
                Access::STORE,
                Policy::NONE
            )
            .is_err(),
            "activation copy unexpectedly transferable"
        );
        assert!(
            crate::service::hub::bridge::activate(hub, &[coalition]).is_err(),
            "activation accepted a non-Hub kernel sender"
        );
        finished.store(true, Ordering::Release);
    });
    port::ship(
        &HolePie::from_token(entry),
        caller.id(),
        Access::STORE,
        Policy::NONE,
    )
    .unwrap();
    let until = env::chrono::clock() + 5_000_000_000;
    while !done.load(Ordering::Acquire) {
        crate::service::hub::bridge::maintain(
            assembly.resources.read().unwrap(),
            assembly.resources.read().unwrap(),
            assembly.resources.read().unwrap(),
        )
        .unwrap();
        assert!(
            env::chrono::clock() < until,
            "activation boundary never answered"
        );
        adapt::sleep(core::time::Duration::from_millis(1)).unwrap();
    }
    caller.join();
}

fn ready_driver(
    operator: &protocol::system::operator::client::Face,
    query: &protocol::system::identity::client::TaskQuery,
    task: env::TaskId,
    road: &'static str,
) {
    let until = env::chrono::clock() + 5_000_000_000;
    loop {
        if let Ok(entry) = operator
            .tile(protocol::common::path::Path::new(road), Wait::AtMost(1000))
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
        adapt::sleep(core::time::Duration::from_millis(1)).unwrap();
    }
}
