//! Real IPC authorized construction and instance failures exercised against an isolated Control fixture.
use super::fixture::Fixture;
use crate::system::control::{
    core::unit::{Slot, State as UnitState},
    serve,
};
use alloc::sync::Arc;
use core::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use env::{Permission, TaskId, Wait, unit};
use protocol::common::schedule::{Cursor, Progress, Schedule};
use protocol::system::control::{self, Face, State, account::Client};

const WAIT: Wait = Wait::AtMost(2000);
struct Signals {
    stage: AtomicUsize,
    ack: AtomicUsize,
    target: AtomicUsize,
    peer_done: AtomicBool,
    done: AtomicBool,
    entry: AtomicUsize,
}
fn until(mut condition: impl FnMut() -> bool) {
    let deadline = env::chrono::clock() + 12_000_000_000;
    while !condition() {
        assert!(
            env::chrono::clock() < deadline,
            "account-instance: worker timed out"
        );
        runtime::core::task::sleep(core::time::Duration::from_millis(1)).unwrap();
    }
}
fn announce(signals: &Signals, stage: usize, task: TaskId) {
    signals.target.store(task.get(), Ordering::Release);
    signals.stage.store(stage, Ordering::Release);
    until(|| signals.ack.load(Ordering::Acquire) == stage);
}
fn closed(client: &Face, task: TaskId) {
    until(|| client.instance(task).state(WAIT).unwrap() == State::Dead);
}
fn reference(root: TaskId) -> control::Object {
    let entry =
        protocol::communication::session::establish::find(root, control::publication::REF).unwrap();
    let authority = runtime::core::res::pie::pies()
        .find(|pie| pie.mark == protocol::system::identity::Grant::Resolve.mark())
        .unwrap()
        .owner;
    control::Client::reference_direct(root, authority, entry, 1, "anran", WAIT).unwrap()
}
pub fn acceptance(assembly: &mut Fixture, operator: &protocol::system::operator::Face) {
    let root = unit::self_id();
    let heirs = unit::heir_count();
    let signals = Arc::new(Signals {
        stage: AtomicUsize::new(0),
        ack: AtomicUsize::new(0),
        target: AtomicUsize::new(0),
        peer_done: AtomicBool::new(false),
        done: AtomicBool::new(false),
        entry: AtomicUsize::new(0),
    });
    let s = signals.clone();
    let worker = runtime::core::task::join::closure(move || {
        until(|| s.entry.load(Ordering::Acquire) != 0);
        let client = Client::of(
            protocol::communication::session::establish::find(root, control::account::ENTRY)
                .unwrap(),
        )
        .unwrap();
        let lifecycle = Face::of(
            protocol::communication::session::establish::find(root, control::ASK_MARK).unwrap(),
        )
        .unwrap();
        assert_eq!(
            client.create("unknown", WAIT).err(),
            Some(control::Fail::Unknown)
        );
        let account = reference(root);
        let first = client.create("anran", WAIT).unwrap();
        announce(&s, 1, first.task);
        until(|| s.peer_done.load(Ordering::Acquire));
        lifecycle.instance(first.task).ruin(WAIT).unwrap();
        assert_eq!(
            lifecycle.instance(first.task).state(WAIT).unwrap(),
            State::Dead
        );
        let expired = client.create("anran", WAIT).unwrap();
        assert_ne!(first.task, expired.task);
        assert_ne!(first.team, expired.team);
        announce(&s, 2, expired.task);
        closed(&lifecycle, expired.task);
        assert_eq!(
            lifecycle.instance(expired.task).embark(WAIT),
            Err(control::Fail::NotReady)
        );
        assert_eq!(
            lifecycle.instance(expired.task).state(WAIT).unwrap(),
            State::Dead
        );
        announce(&s, 12, expired.task);
        lifecycle.instance(expired.task).ruin(WAIT).unwrap();
        let failed = client.create("anran", WAIT).unwrap();
        announce(&s, 3, failed.task);
        assert!(matches!(
            lifecycle.instance(failed.task).embark(WAIT),
            Err(control::Fail::NotReady)
        ));
        closed(&lifecycle, failed.task);
        announce(&s, 13, failed.task);
        lifecycle.instance(failed.task).ruin(WAIT).unwrap();
        assert_eq!(reference(root), account);
        let abandoned = client.create("anran", WAIT).unwrap();
        announce(&s, 4, abandoned.task);
        s.done.store(true, Ordering::Release);
        // Exiting with a unclaimed instance exercises automatic owner-death cleanup.
    });
    let s = signals.clone();
    let entry = assembly
        .resources
        .read::<serve::account::Accounts>()
        .unwrap()
        .entry;
    let alias = operator
        .tile(
            protocol::common::path::Path::new("/idt/principal/anran/ref"),
            WAIT,
        )
        .unwrap()
        .token(WAIT)
        .unwrap();
    env::pie::accord(
        alias,
        worker.id(),
        Permission::FETCH | Permission::STORE,
        control::publication::REF,
    )
    .unwrap();
    let peer_entry = env::pie::accord(
        entry,
        worker.id(),
        Permission::FETCH | Permission::STORE,
        control::account::ENTRY,
    )
    .unwrap();
    let instance_entry = assembly
        .resources
        .read::<serve::watch::Watch>()
        .unwrap()
        .instance
        .unwrap();
    env::pie::accord(
        instance_entry,
        worker.id(),
        Permission::FETCH | Permission::STORE,
        control::ASK_MARK,
    )
    .unwrap();
    let p = signals.clone();
    let peer_token = Arc::new(AtomicUsize::new(0));
    let token = peer_token.clone();
    let peer = runtime::core::task::join::closure(move || {
        until(|| p.stage.load(Ordering::Acquire) == 1 && token.load(Ordering::Acquire) != 0);
        let client = Client::of(
            protocol::communication::session::establish::find(root, control::account::ENTRY)
                .unwrap(),
        )
        .unwrap();
        let lifecycle = Face::of(
            protocol::communication::session::establish::find(root, control::ASK_MARK).unwrap(),
        )
        .unwrap();
        let target = TaskId::new(p.target.load(Ordering::Acquire));
        assert!(matches!(
            client.create("anran", WAIT),
            Err(control::Fail::Denied)
        ));
        assert!(matches!(
            lifecycle.instance(target).embark(WAIT),
            Err(control::Fail::Denied)
        ));
        assert!(matches!(
            lifecycle.instance(target).state(WAIT),
            Err(control::Fail::Denied)
        ));
        assert!(matches!(
            lifecycle.instance(target).ruin(WAIT),
            Err(control::Fail::Denied)
        ));
        p.peer_done.store(true, Ordering::Release);
        until(|| p.done.load(Ordering::Acquire));
    });
    let peer_grant = env::pie::accord(
        entry,
        peer.id(),
        Permission::FETCH | Permission::STORE,
        control::account::ENTRY,
    )
    .unwrap();
    env::pie::accord(
        instance_entry,
        peer.id(),
        Permission::FETCH | Permission::STORE,
        control::ASK_MARK,
    )
    .unwrap();
    peer_token.store(peer_grant.get(), Ordering::Release);
    {
        let mut system = assembly.resources.write::<serve::unit::Control>().unwrap();
        system.enlist(&crate::unit::login::PROGRAM).unwrap();
        system
            .table
            .attach(
                "login",
                Slot::Live {
                    task: worker.id(),
                    team: None,
                },
            )
            .unwrap();
        system.table.set_state("login", UnitState::Ready);
        system.enlist(&crate::unit::terminal::PROGRAM).unwrap();
        system
            .table
            .attach(
                "terminal",
                Slot::Live {
                    task: peer.id(),
                    team: None,
                },
            )
            .unwrap();
        system.table.set_state("terminal", UnitState::Ready);
    }
    let login_subject = {
        let roster = assembly
            .resources
            .read::<crate::system::identity::serve::install::Roster>()
            .unwrap();
        roster.inherit(worker.id(), root).unwrap();
        roster.inherit(peer.id(), root).unwrap();
        crate::system::identity::serve::query::binding(&roster, worker.id())
            .unwrap()
            .unwrap()
            .current
    };
    let mut schedule = Schedule::new();
    schedule
        .add_system("account.receive", 0u8, serve::account::receive)
        .unwrap();
    schedule
        .add_system("instances.receive", 1, serve::instance::receive)
        .unwrap();
    schedule
        .add_system("instances.answer", 2, serve::instance::answer)
        .unwrap();
    schedule
        .add_system("instances.reap", 3, serve::instance::reap)
        .unwrap();
    schedule
        .add_system("account.completed", 4, serve::account::completed)
        .unwrap();
    let mut plan = schedule.build().unwrap();
    plan.prepare(&assembly.resources);
    let mut cursor = Cursor::default();
    s.entry.store(peer_entry.get(), Ordering::Release);
    let deadline = env::chrono::clock() + 15_000_000_000;
    let mut observed = 0;
    let mut user = None;
    loop {
        assembly.progress().unwrap();
        if plan.advance(&mut cursor, &assembly.resources).unwrap() == Progress::Done {
            cursor.reset();
        }
        let stage = signals.stage.load(Ordering::Acquire);
        let target = TaskId::new(signals.target.load(Ordering::Acquire));
        if stage != 0 && stage != observed {
            let roster = assembly
                .resources
                .read::<crate::system::identity::serve::install::Roster>()
                .unwrap();
            let binding = crate::system::identity::serve::query::binding(&roster, target).unwrap();
            let runtime = assembly
                .resources
                .read::<serve::resource::Resources>()
                .unwrap()
                .runtime_road(target);
            if stage < 10 {
                let subject = binding
                    .expect("account-instance: Held task has no identity")
                    .current;
                assert_ne!(subject.principal, login_subject.principal);
                assert!(
                    runtime.is_some(),
                    "account-instance: create returned before runtime registration"
                );
                if let Some(principal) = user {
                    assert_eq!(principal, subject.principal);
                }
                user = Some(subject.principal);
                if stage == 3 {
                    env::room::doom(target).unwrap();
                }
            } else {
                assert!(binding.is_none(), "account-instance: Dead task still bound");
                assert!(
                    runtime.is_none(),
                    "account-instance: Dead task still registered"
                );
            }
            observed = stage;
            signals.ack.store(stage, Ordering::Release);
        }
        let reclaimed = assembly
            .resources
            .read::<serve::unit::Control>()
            .unwrap()
            .instances
            .iter()
            .all(|item| item.team.is_none());
        let registered = assembly
            .resources
            .read::<serve::resource::Resources>()
            .unwrap()
            .runtime_road(target)
            .is_some();
        if signals.done.load(Ordering::Acquire)
            && unit::join(worker.id(), Wait::POLL).unwrap_or(true)
            && reclaimed
            && !registered
        {
            break;
        }
        assert!(
            env::chrono::clock() < deadline,
            "account-instance: fixture timed out"
        );
        runtime::core::task::sleep(core::time::Duration::from_millis(1)).unwrap();
    }
    let worker_id = worker.id();
    let peer_id = peer.id();
    worker.join();
    peer.join();
    {
        let roster = assembly
            .resources
            .read::<crate::system::identity::serve::install::Roster>()
            .unwrap();
        roster.unbind(worker_id).unwrap();
        roster.unbind(peer_id).unwrap();
        let target = TaskId::new(signals.target.load(Ordering::Acquire));
        assert!(
            crate::system::identity::serve::query::binding(&roster, target)
                .unwrap()
                .is_none()
        );
    }
    {
        let mut system = assembly.resources.write::<serve::unit::Control>().unwrap();
        system.table.detach("login");
        system.table.set_state("login", UnitState::Dead);
        system.table.detach("terminal");
        system.table.set_state("terminal", UnitState::Dead);
    }
    assembly.progress().unwrap();
    assert!(
        operator
            .tile(
                protocol::common::path::Path::new("/idt/principal/anran/ref"),
                WAIT
            )
            .unwrap()
            .token(WAIT)
            .is_ok()
    );
    assert_eq!(
        unit::heir_count(),
        heirs,
        "account-instance: runtime team leaked"
    );
    protocol::debug::put(
        "account-instance: foreign caller, expiry, start failure and owner death cleaned",
    );
}
