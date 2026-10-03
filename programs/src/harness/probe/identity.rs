//! Trusted setup for the two Identity probes, not a product capability declaration.

use env::Wait;
use protocol::communication::session::establish;
use protocol::system::identity::Grant;
use runtime::core::res::port::{self, Access, Policy};
use runtime::env::mail::{self, HolePie};

use crate::harness::probe::fixture::Fixture;
use crate::unit::{self, UnitFile};

pub(crate) fn supply_to(
    authority: Option<env::TaskId>,
    program: &UnitFile,
    task: env::TaskId,
) -> Result<(), &'static str> {
    if program.name() == "system-dependent" { super::hierarchy::supply(task)?; }
    if program.name() != unit::probe_denied::PROBE_DENIED.name()
        && program.name() != unit::probe_coalition::PROBE_COALITION.name()
    {
        return Ok(());
    }
    let authority = authority.ok_or("identity fixture authority")?;
    for grant in [Grant::Bind, Grant::Unbind] {
        let token = establish::claim(authority, grant.mark(), Wait::POLL)
            .ok_or("identity fixture face")?;
        if !matches!(mail::reserve(token), Ok((_, owner, mark))
            if owner == authority && mark == grant.mark())
        {
            return Err("identity fixture source");
        }
        // Transfer an entry copy, not the installer's kernel sender identity.
        port::ship(&HolePie::from_token(token), task,
            Access::FETCH | Access::STORE, Policy::NONE)
            .map_err(|_| "identity fixture transfer")?;
    }
    Ok(())
}

/// A timed-out client may drop its send buffer while the kernel still holds the request.
pub fn timeout() {
    use alloc::sync::Arc;
    use core::sync::atomic::{AtomicBool, Ordering};
    use protocol::system::identity::{Wire, client::{CallError, Face}, limits::MAX_FRAME};

    let owner = runtime::env::unit::self_id();
    let entry = mail::unseal_hole(Grant::Resolve.mark()).unwrap();
    let timed_out = Arc::new(AtomicBool::new(false));
    let release_reader = timed_out.clone();
    let raw_owner = owner.get();
    let reader = runtime::core::task::join::closure(move || {
        let owner = env::TaskId::new(raw_owner);
        let entry = establish::claim(owner, Grant::Resolve.mark(), Wait::AtMost(1000)).unwrap();
        while !release_reader.load(Ordering::Acquire) {
            runtime::env::room::sleep(core::time::Duration::from_millis(1)).unwrap();
        }
        let mut bytes = [0; MAX_FRAME];
        let (n, from) = HolePie::from_token(entry).pull(&mut bytes, Wait::AtMost(1000)).unwrap();
        assert_eq!(from, owner);
        assert_eq!(Wire::take(&bytes[..n]).unwrap().0, Some(Wire::Resolve(owner)));
    });
    port::ship(&HolePie::from_token(entry), reader.id(), Access::FETCH, Policy::NONE).unwrap();
    let face = Face::direct(owner, Grant::Resolve, entry).unwrap();
    let result = face.call(Wire::Resolve(owner), Wait::AtMost(1));
    timed_out.store(true, Ordering::Release);
    assert_eq!(result, Err(CallError::Transport));
    reader.join();
    let _ = mail::seal(entry);
    let _ = mail::release(entry);
    protocol::debug::put("identity-timeout: queued request decoded after client timeout");
}

/// Isolated supervisor fixture. Every observation goes over the real Identity IPC faces.
pub fn acceptance() {

    use protocol::system::identity::client::TaskQuery;
    use crate::system::run::{bootstrap, scene};

    super::hierarchy::codecs();
    super::hierarchy::reference_lifetime();
    let boot = bootstrap::take().expect("identity: bootstrap");
    let list = scene::programs(&boot.catalog).expect("identity: scene");
    let mut assembly = Fixture::new(boot).ok().expect("identity: assembly");
    for program in list {
        if program.name() == "system-child" {
            assembly.control.enlist(program).expect("identity: runtime declaration");
            continue;
        }
        assembly.assemble(program).expect("identity: install static unit");
    }
    let query = |authority| {
        let face = |grant: Grant| establish::find(authority, grant.mark())
            .expect("identity: face missing");
        TaskQuery::direct(authority, face(Grant::Resolve), face(Grant::Matches), face(Grant::Same))
            .expect("identity: face source")
    };
    for name in ["operator", "identity"] {
        assert!(assembly.control.mint(name.into(), &assembly.images).is_err());
        assert!(assembly.control.stop(name.into(), &assembly.roster, &mut assembly.activation).is_err());
        assert_eq!(assembly.control.state(name.into()).unwrap(), crate::system::control::core::unit::State::Ready);
    }
    let old_authority = assembly.roster.authority().unwrap();
    let old_query = query(old_authority);
    let me = runtime::env::unit::self_id();
    let host = assembly.tree.host().unwrap();
    let link = establish::endpoint(host, env::Mark::of(protocol::system::operator::LINK), Wait::POLL)
        .expect("identity: operator request");
    let talk = establish::give(host, protocol::system::operator::ASK_MARK)
        .expect("identity: operator ask");
    let tip = establish::find(host, protocol::system::operator::TIP_MARK)
        .expect("identity: trusted operator tip");
    let mut record = [0u8; protocol::system::operator::TIP_LEN];
    let n = protocol::system::operator::Tip::Guest(me).store(&mut record).unwrap();
    HolePie::from_token(tip).push(&record[..n], Wait::AtMost(1000))
        .expect("identity: trusted guest registration");
    let operator = protocol::system::operator::client::Face::of(
        protocol::communication::session::Session { link, talk, host });
    let protected = || operator.tile(protocol::common::path::Path::new("/svc/sys/control/mint"),
        Wait::AtMost(1000)).expect("identity: protected tile");
    assert!(protected().token(Wait::AtMost(1000)).is_ok(),
        "identity: trusted operation");
    let public = operator.tile(protocol::common::path::Path::new("/svc/sys/control/state"),
        Wait::AtMost(1000)).expect("identity: public tile");
    assert!(public.token(Wait::AtMost(1000)).is_ok());
    let old = old_query.resolve(me, Wait::AtMost(1000)).unwrap().unwrap();
    let old_dependent = assembly.control.task("system-dependent").unwrap();
    let old_hub = assembly.control.task("hub").unwrap();
    let old_devices = old_query.resolve(old_hub, Wait::AtMost(1000)).unwrap().unwrap().current;
    assert!(!old_devices.coalitions.is_empty(), "identity: Hub selections not activated");
    let coalition = old_devices.coalitions.iter().next().unwrap();
    activation_boundary(&assembly, old_hub, coalition);
    let before = old_query.resolve(old_dependent, Wait::AtMost(1000)).unwrap();
    assert!(assembly.roster.activate(old_dependent, coalition).is_err(),
        "identity: qualification was not checked");
    assert_eq!(old_query.resolve(old_dependent, Wait::AtMost(1000)).unwrap(), before);
    for (name, road) in [("router", "/svc/drv/router"), ("rtc", "/svc/drv/rtc"),
        ("uart", "/svc/drv/uart/rx")]
    {
        ready_driver(&operator, &old_query, assembly.control.task(name).unwrap(), road);
    }
    assembly.control.mint(alloc::string::String::from("system-child"), &assembly.images)
        .expect("identity: runtime mint");
    let machine = assembly.supplies.machine;
    let child = assembly.control.release(alloc::string::String::from("system-child"),
        old_dependent, &assembly.roster, &mut assembly.activation, &mut assembly.supplies, |control, activation| crate::system::run::cycle::poll(control, &assembly.roster, &machine, activation, assembly.images.entry, &mut assembly.publications, &mut assembly.runtime, &mut assembly.names, &mut assembly.tree))
        .expect("identity: runtime inheritance").0;
    let parent_binding = old_query.resolve(old_dependent, Wait::AtMost(1000)).unwrap().unwrap();
    let child_binding = old_query.resolve(child, Wait::AtMost(1000)).unwrap().unwrap();
    assert_eq!(child_binding.origin, parent_binding.current);
    assert_eq!(old.current.principal.authority, old_authority);
    let dynamic = super::hierarchy::exercise(&mut assembly, &operator, old_authority, old_dependent, child);
    let _ = dynamic;
    protocol::debug::put("system: identity, device and publication acceptance passed");
    assembly.control.stop_rest(&assembly.roster, &mut assembly.activation);
    assert!(assembly.supervise().is_ok(), "system: normal team shutdown failed");
}

fn activation_boundary(assembly: &Fixture, hub: env::TaskId,
    coalition: protocol::system::identity::CoalitionId)
{
    use alloc::sync::Arc;
    use core::sync::atomic::{AtomicBool, Ordering};
    use protocol::service::hub::activation;

    let owner = runtime::env::unit::self_id();
    let entry = establish::find(owner, activation::ENTRY).unwrap();
    let done = Arc::new(AtomicBool::new(false));
    let finished = done.clone();
    let (owner, hub, authority, slot) = (owner.get(), hub.get(), coalition.authority.get(), coalition.slot);
    let caller = runtime::core::task::join::closure(move || {
        let owner = env::TaskId::new(owner);
        let hub = env::TaskId::new(hub);
        let coalition = protocol::system::identity::CoalitionId::new(env::TaskId::new(authority), slot);
        let entry = establish::claim(owner, activation::ENTRY, Wait::AtMost(1000)).unwrap();
        assert!(port::ship(&HolePie::from_token(entry), hub, Access::STORE, Policy::NONE).is_err(),
            "activation copy unexpectedly transferable");
        assert!(crate::service::hub::bridge::activate(hub, &[coalition]).is_err(),
            "activation accepted a non-Hub kernel sender");
        finished.store(true, Ordering::Release);
    });
    port::ship(&HolePie::from_token(entry), caller.id(), Access::STORE, Policy::NONE).unwrap();
    let until = runtime::env::chrono::clock() + 5_000_000_000;
    while !done.load(Ordering::Acquire) {
        if let Some(activation) = &assembly.activation { activation.poll(&assembly.control, &assembly.roster); }
        assert!(runtime::env::chrono::clock() < until, "activation boundary never answered");
        runtime::env::room::sleep(core::time::Duration::from_millis(1)).unwrap();
    }
    caller.join();
}

fn ready_driver(
    operator: &protocol::system::operator::client::Face,
    query: &protocol::system::identity::client::TaskQuery,
    task: env::TaskId,
    road: &'static str,
) {
    let until = runtime::env::chrono::clock() + 5_000_000_000;
    loop {
        if let Ok(entry) = operator.tile(protocol::common::path::Path::new(road), Wait::AtMost(1000))
            .and_then(|tile| tile.token(Wait::AtMost(1000)))
        {
            if matches!(mail::inspect(entry), Ok((_, owner, _)) if owner == task) {
                let binding = query.resolve(task, Wait::AtMost(1000)).unwrap().unwrap();
                assert!(!binding.current.coalitions.is_empty(), "identity: driver inactive");
                assert_eq!(binding.current.principal.authority, query.authority());
                return;
            }
        }
        assert!(runtime::env::chrono::clock() < until, "identity: driver never republished");
        runtime::env::room::sleep(core::time::Duration::from_millis(1)).unwrap();
    }
}
