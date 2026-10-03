//! Trusted setup for the two Identity probes, not a product capability declaration.

use env::Wait;
use protocol::communication::session::establish;
use protocol::service::identity::Grant;
use runtime::core::res::port::{self, Access, Policy};
use runtime::env::mail::{self, HolePie};

use crate::system::Assembly;
use crate::system::control::Service;
use crate::unit::{self, UnitFile};

pub(crate) fn supply(
    assembly: &mut Assembly,
    program: &UnitFile,
    service: &mut Service,
) -> Result<(), &'static str> {
    supply_to(assembly.control.roster.authority(), program, service.0)
}

pub(crate) fn supply_to(
    authority: Option<env::TaskId>,
    program: &UnitFile,
    task: env::TaskId,
) -> Result<(), &'static str> {
    if program.name() == "replacement-dependent" { super::hierarchy::supply(task)?; }
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
    use protocol::service::identity::{Wire, client::{CallError, Face}, limits::MAX_FRAME};

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
pub fn replacement() {
    use protocol::service::identity::{Fail, Match, Selector};
    use protocol::service::identity::client::{CallError, TaskQuery};
    use crate::system::run::{bootstrap, scene};

    super::hierarchy::codecs();
    super::hierarchy::reference_lifetime();
    let boot = bootstrap::take().expect("replacement: bootstrap");
    let list = scene::programs(&boot.catalog).expect("replacement: scene");
    let mut assembly = Assembly::new(boot).ok().expect("replacement: assembly");
    for program in list {
        if program.name() == "replacement-child" {
            assembly.control.enlist(program).expect("replacement: runtime declaration");
            continue;
        }
        assembly.assemble(program).expect("replacement: install static unit");
    }
    assembly.mount_control();
    let query = |authority| {
        let face = |grant: Grant| establish::find(authority, grant.mark())
            .expect("replacement: face missing");
        TaskQuery::direct(authority, face(Grant::Resolve), face(Grant::Matches), face(Grant::Same))
            .expect("replacement: face source")
    };
    let old_authority = assembly.control.roster.authority().unwrap();
    let old_query = query(old_authority);
    let me = runtime::env::unit::self_id();
    let host = assembly.tree.host().unwrap();
    let link = establish::endpoint(host, env::Mark::of(protocol::service::operator::LINK), Wait::POLL)
        .expect("replacement: operator request");
    let talk = establish::give(host, protocol::service::operator::ASK_MARK)
        .expect("replacement: operator ask");
    let tip = establish::find(host, protocol::service::operator::TIP_MARK)
        .expect("replacement: trusted operator tip");
    let mut record = [0u8; protocol::service::operator::TIP_LEN];
    let n = protocol::service::operator::Tip::Guest(me).store(&mut record).unwrap();
    HolePie::from_token(tip).push(&record[..n], Wait::AtMost(1000))
        .expect("replacement: trusted guest registration");
    let operator = protocol::service::operator::client::Face::of(
        protocol::communication::session::Session { link, talk, host });
    let protected = || operator.tile(protocol::common::path::Path::new("/svc/sys/control/mint"),
        Wait::AtMost(1000)).expect("replacement: protected tile");
    assert!(protected().token(Wait::AtMost(1000)).is_ok(),
        "replacement: trusted operation before kill");
    let public = operator.tile(protocol::common::path::Path::new("/svc/sys/control/state"),
        Wait::AtMost(1000)).expect("replacement: public tile");
    let old = old_query.resolve(me, Wait::AtMost(1000)).unwrap().unwrap();
    let old_dependent = assembly.control.task("replacement-dependent").unwrap();
    let old_hub = assembly.control.task("hub").unwrap();
    let old_devices = old_query.resolve(old_hub, Wait::AtMost(1000)).unwrap().unwrap().current;
    assert!(!old_devices.coalitions.is_empty(), "replacement: Hub selections not activated");
    let coalition = old_devices.coalitions.iter().next().unwrap();
    activation_boundary(&assembly, old_hub, coalition);
    let before = old_query.resolve(old_dependent, Wait::AtMost(1000)).unwrap();
    assert!(assembly.control.roster.activate(old_dependent, coalition).is_err(),
        "replacement: qualification was not checked");
    assert_eq!(old_query.resolve(old_dependent, Wait::AtMost(1000)).unwrap(), before);
    for (name, road) in [("router", "/svc/drv/router"), ("rtc", "/svc/drv/rtc"),
        ("uart", "/svc/drv/uart/rx")]
    {
        ready_driver(&operator, &old_query, assembly.control.task(name).unwrap(), road);
    }
    assembly.control.mint(alloc::string::String::from("replacement-child"))
        .expect("replacement: runtime mint");
    let child = assembly.control.release(alloc::string::String::from("replacement-child"),
        old_dependent, |control| control.progress(&mut assembly.tree))
        .expect("replacement: runtime inheritance").0;
    let parent_binding = old_query.resolve(old_dependent, Wait::AtMost(1000)).unwrap().unwrap();
    let child_binding = old_query.resolve(child, Wait::AtMost(1000)).unwrap().unwrap();
    assert_eq!(child_binding.origin, parent_binding.current);
    assert_eq!(old.current.principal.authority, old_authority);
    let dynamic = super::hierarchy::exercise(&mut assembly, &operator, old_authority, old_dependent, child);
    runtime::env::room::doom(old_authority).expect("replacement: kill old authority");
    assert!(runtime::env::unit::join(old_authority, Wait::AtMost(1000)).unwrap());
    assert!(old_query.resolve(me, Wait::AtMost(1000)).is_err(),
        "replacement: old authority still serves");
    assert!(!old_query.available(), "replacement: sealed source bundle still available");
    assert_eq!(protected().token(Wait::AtMost(1000)),
        Err(protocol::service::operator::Fail::Unjudged));
    assert!(public.token(Wait::AtMost(1000)).is_ok(),
        "replacement: public discovery blocked by identity outage");
    assert!(assembly.watch.recover_identity(&mut assembly.control, &mut assembly.tree)
        .expect("replacement: trusted reinstall"));
    let authority = assembly.control.roster.authority().unwrap();
    assert_ne!(authority, old_authority);
    let fresh = query(authority);
    assert!(fresh.available());
    assert!(protected().token(Wait::AtMost(1000)).is_ok(),
        "replacement: Control permit was not refreshed");
    assert_eq!(fresh.matches(me, Selector::Exact(old.current.principal), Wait::AtMost(1000)),
        Err(CallError::Service(Fail::WrongAuthority)));
    for task in [me, assembly.tree.host().unwrap(), authority,
        assembly.control.task("replacement-dependent").unwrap()]
    {
        let binding = fresh.resolve(task, Wait::AtMost(1000)).unwrap().unwrap();
        assert_eq!(binding.current.principal.authority, authority);
        assert_eq!(binding.origin, binding.current);
        assert_eq!(fresh.matches(task, Selector::Exact(binding.current.principal),
            Wait::AtMost(1000)), Ok(Match::Yes));
    }
    assert_ne!(assembly.control.task("replacement-dependent").unwrap(), old_dependent);
    assert!(runtime::env::unit::join(old_dependent, Wait::POLL).unwrap_or(true));
    assert!(assembly.control.task("replacement-child").is_none_or(|known| known == child
        && runtime::env::unit::join(known, Wait::POLL).unwrap_or(true)),
        "replacement: runtime child was restarted as a static root");
    assert_eq!(fresh.resolve(child, Wait::AtMost(1000)), Ok(None));
    let hub = assembly.control.task("hub").unwrap();
    assert_ne!(hub, old_hub);
    let devices = fresh.resolve(hub, Wait::AtMost(1000)).unwrap().unwrap().current;
    assert_eq!(devices.coalitions.len(), old_devices.coalitions.len());
    for coalition in devices.coalitions.iter() {
        assert_eq!(coalition.authority, authority);
        assert_eq!(fresh.matches(hub, Selector::MemberOf(coalition), Wait::AtMost(1000)),
            Ok(Match::Yes));
    }
    for (name, road) in [("router", "/svc/drv/router"), ("rtc", "/svc/drv/rtc"),
        ("uart", "/svc/drv/uart/rx")]
    {
        ready_driver(&operator, &fresh, assembly.control.task(name).unwrap(), road);
    }
    super::hierarchy::after_replacement(&mut assembly, &operator, old_authority, authority, dynamic);
    protocol::debug::put("identity-replacement: Hub selections and three device drivers restored");
    protocol::debug::put("identity-replacement: old IDs rejected; new source and static restart verified");
    assembly.control.stop_rest();
}

fn activation_boundary(assembly: &Assembly, hub: env::TaskId,
    coalition: protocol::service::identity::CoalitionId)
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
        let coalition = protocol::service::identity::CoalitionId::new(env::TaskId::new(authority), slot);
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
        assembly.control.activate_hub();
        assert!(runtime::env::chrono::clock() < until, "activation boundary never answered");
        runtime::env::room::sleep(core::time::Duration::from_millis(1)).unwrap();
    }
    caller.join();
}

fn ready_driver(
    operator: &protocol::service::operator::client::Face,
    query: &protocol::service::identity::client::TaskQuery,
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
                assert!(!binding.current.coalitions.is_empty(), "replacement: driver inactive");
                assert_eq!(binding.current.principal.authority, query.authority());
                return;
            }
        }
        assert!(runtime::env::chrono::clock() < until, "replacement: driver never republished");
        runtime::env::room::sleep(core::time::Duration::from_millis(1)).unwrap();
    }
}
