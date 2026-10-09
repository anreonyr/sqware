//! Real IPC authorized construction and instance failures exercised against an isolated Control fixture.
use super::fixture::Fixture;
use crate::system::app::Fault as Fail;
use crate::system::control::{instance::hook, unit::Control, unit::table::State as UnitState};

use ::schedule::{Cursor, Progress, Schedule};
use alloc::sync::Arc;
use core::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use env::{Permission, TaskId, Wait, unit};
use system_api::control;
use system_api::control::State;
use system_client::control::Face;
use account_client::Client;

const WAIT: Wait = Wait::AtMost(2000);
#[derive(Default)]
struct HookFault {
    prepare: bool,
    failed_task: Option<TaskId>,
    waited: bool,
    retired_failed: bool,
    waiting: Option<usize>,
    signals: Option<Arc<Signals>>,
}
fn fail_prepare(
    mut fault: ::schedule::ResMut<HookFault>,
    active: ::schedule::Res<hook::Active>,
    resources: ::schedule::Res<crate::system::publication::RuntimeNamespace>,
) -> Result<Progress, &'static str> {
    if fault.prepare {
        let task = active.task.ok_or("probe hook target")?;
        assert!(
            resources.runtime_road(task).is_some(),
            "prepare hook failed before mounting runtime"
        );
        fault.prepare = false;
        fault.failed_task = Some(task);
        return Err("probe preparation hook failure");
    }
    Ok(Progress::Done)
}
fn delay_retire(
    mut fault: ::schedule::ResMut<HookFault>,
    active: ::schedule::Res<hook::Active>,
    control: ::schedule::Res<Control>,
) -> Result<Progress, &'static str> {
    let item = control
        .instances()
        .find(|item| Some(item.task) == active.task)
        .unwrap();
    assert_eq!(item.state, UnitState::Stopping);
    assert!(
        item.team.is_some(),
        "retire hook reported reclaimed before completion"
    );
    if !fault.waited {
        fault.waited = true;
        fault.waiting = Some(
            fault
                .signals
                .as_ref()
                .unwrap()
                .pings
                .load(Ordering::Acquire),
        );
        return Ok(Progress::Pending);
    }
    if let Some(baseline) = fault.waiting {
        if fault
            .signals
            .as_ref()
            .unwrap()
            .pings
            .load(Ordering::Acquire)
            <= baseline
        {
            return Ok(Progress::Pending);
        }
        fault.waiting = None;
    }
    if !fault.retired_failed {
        fault.retired_failed = true;
        return Err("probe retirement hook failure");
    }
    Ok(Progress::Done)
}
fn hooks() -> ::schedule::Plan<Fail> {
    let children = crate::system::launch::hooks::children().unwrap();
    let mut wrapped = alloc::vec::Vec::new();
    for (key, child) in children {
        let mut wrap = Schedule::new();
        if key == hook::Key::Retire {
            wrap.add_system("delay", 0u8, delay_retire).unwrap();
        }
        wrap.add_plan("registered", 1u8, child).unwrap();
        if key == hook::Key::Prepare {
            wrap.add_system("fail", 2u8, fail_prepare).unwrap();
        }
        wrapped.push((key, wrap.build().unwrap()));
    }
    hook::plan(wrapped).unwrap()
}
struct Signals {
    pings: AtomicUsize,
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
        execution::room::park(core::time::Duration::from_millis(1)).unwrap();
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
    let entry = ipc::session::establish::find(root, control::publication::REF).unwrap();
    let authority = ::resource::raw::pies()
        .find(|pie| pie.mark == system_api::identity::Grant::Resolve.mark())
        .unwrap()
        .owner;
    system_client::control::publication::Client::reference_direct(
        root, authority, entry, 1, "anran", WAIT,
    )
    .unwrap()
}
pub(crate) fn acceptance(assembly: &mut Fixture, operator: &system_client::operator::Face) {
    let root = unit::self_id();
    let heirs = unit::heir_count();
    let account_task = assembly.resources.read::<crate::system::app::account::Account>().unwrap().task.unwrap();
    let signals = Arc::new(Signals {
        pings: AtomicUsize::new(0),
        stage: AtomicUsize::new(0),
        ack: AtomicUsize::new(0),
        target: AtomicUsize::new(0),
        peer_done: AtomicBool::new(false),
        done: AtomicBool::new(false),
        entry: AtomicUsize::new(0),
    });
    let s = signals.clone();
    let worker = execution::unit::task::spawn(move || {
        until(|| s.entry.load(Ordering::Acquire) != 0);
        let client =
            Client::of(ipc::session::establish::find(account_task, account_api::ENTRY).unwrap())
                .unwrap();
        let lifecycle =
            Face::of(ipc::session::establish::find(root, control::ASK_MARK).unwrap()).unwrap();
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
        announce(&s, 5, TaskId::new(0));
        assert_eq!(
            client.create("anran", WAIT).err(),
            Some(control::Fail::NotReady)
        );
        announce(&s, 15, TaskId::new(0));
        let abandoned = client.create("anran", WAIT).unwrap();
        announce(&s, 4, abandoned.task);
        s.done.store(true, Ordering::Release);
        // Exiting with a unclaimed instance exercises automatic owner-death cleanup.
    });
    let s = signals.clone();
    let token = assembly.resources.read::<crate::system::app::account::Account>().unwrap().config.entry.load(Ordering::Acquire);
    let entry = env::PieToken::from_bytes(&(token as u64).to_le_bytes()).unwrap();
    let alias = operator
        .tile(
            system_api::operator::path::Path::new("/idt/principal/anran/ref"),
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
        account_api::ENTRY,
    )
    .unwrap();
    let instance_entry = crate::system::control::instance_entry(&assembly.resources).unwrap();
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
    let peer = execution::unit::task::spawn(move || {
        until(|| p.stage.load(Ordering::Acquire) == 1 && token.load(Ordering::Acquire) != 0);
        let client =
            Client::of(ipc::session::establish::find(account_task, account_api::ENTRY).unwrap())
                .unwrap();
        let lifecycle =
            Face::of(ipc::session::establish::find(root, control::ASK_MARK).unwrap()).unwrap();
        let target = TaskId::new(p.target.load(Ordering::Acquire));
        {
            use system_api::control::construction as api;
            let entry = ipc::session::establish::find(root, api::ENTRY).unwrap();
            let sender = ipc::rpc::request::Sender::<api::Call>::from_raw(entry, api::Call::BACK).unwrap();
            let forged = system_api::identity::Subject::new(system_api::identity::PrincipalId::root(root), &[]).unwrap();
            let said = sender.call(ipc::time::Deadline::new(WAIT), |back| api::Request {
                image: system_api::loader::Ask { op: system_api::loader::BUILD, image: env::PieToken::NONE, offset: 0, len: 0, stack: 0, count: 0, args: [0; system_api::loader::MAX_ARGS], back },
                owner: root, subject: forged,
            }).unwrap();
            assert_eq!(said.status, system_api::control::frame::DENIED, "foreign creator selected another Subject");
        }

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
        while !p.done.load(Ordering::Acquire) {
            assert!(matches!(
                lifecycle.instance(target).state(WAIT),
                Err(control::Fail::Denied)
            ));
            p.pings.fetch_add(1, Ordering::Release);
            execution::room::park(core::time::Duration::from_millis(1)).unwrap();
        }
    });
    let peer_grant = env::pie::accord(
        entry,
        peer.id(),
        Permission::FETCH | Permission::STORE,
        account_api::ENTRY,
    )
    .unwrap();
    env::pie::accord(
        instance_entry,
        peer.id(),
        Permission::FETCH | Permission::STORE,
        control::ASK_MARK,
    )
    .unwrap();
    let construction = assembly.resources.read::<crate::system::control::Construction>().unwrap().entry;
    resource::port::ship(construction, peer.id(), env::Access::STORE, env::Policy::NONE).unwrap();
    peer_token.store(peer_grant.get(), Ordering::Release);
    {
        let mut system = assembly.resources.write::<Control>().unwrap();
        system
            .fixture_attach_unit(&crate::unit::login::PROGRAM, worker.id())
            .unwrap();
        system
            .fixture_attach_unit(&crate::unit::terminal::PROGRAM, peer.id())
            .unwrap();
    }
    {
        let account = assembly.resources.read::<crate::system::app::account::Account>().unwrap();
        account.config.login.store(worker.id().get(), Ordering::Release);
        account.config.terminal.store(peer.id().get(), Ordering::Release);
    }
    let login_subject = {
        let roster = assembly
            .resources
            .read::<crate::system::control::identity::Roster>()
            .unwrap();
        roster.inherit(worker.id(), root).unwrap();
        roster.inherit(peer.id(), root).unwrap();
        crate::system::control::identity::binding(&roster, worker.id())
            .unwrap()
            .unwrap()
            .current
    };
    assembly
        .resources
        .insert(HookFault {
            signals: Some(signals.clone()),
            ..HookFault::default()
        })
        .unwrap();
    let mut schedule = Schedule::new();
    schedule.add_system("account.consumers", 0u8, crate::system::app::account::refresh).unwrap();
    schedule.add_system("construction.receive", 0u8, crate::system::control::receive_construction).unwrap();
    schedule.add_system("construction.admit", 1, crate::system::control::admit_construction).unwrap();
    schedule.add_system("construction.dispatch", 2, crate::system::launch::dispatch).unwrap();
    schedule.add_plan("loader.build", 3, crate::system::loader::frame().unwrap()).unwrap();
    schedule.add_system("launch.register", 4, crate::system::launch::register).unwrap();
    schedule
        .add_system(
            "instances.receive",
            5,
            crate::system::control::receive_instances,
        )
        .unwrap();
    schedule
        .add_system(
            "instances.answer",
            6,
            crate::system::control::answer_instances,
        )
        .unwrap();
    schedule
        .add_system(
            "instances.reap",
            7,
            crate::system::control::instance::schedule::reap,
        )
        .unwrap();
    schedule.add_plan("instance.hooks", 8, hooks()).unwrap();
    schedule
        .add_system("launch.completed", 9, crate::system::launch::completed)
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
        let mut target = TaskId::new(signals.target.load(Ordering::Acquire));
        if stage == 5 && stage != observed {
            assembly.resources.write::<HookFault>().unwrap().prepare = true;
            observed = stage;
            signals.ack.store(stage, Ordering::Release);
            continue;
        }
        if stage == 15 {
            target = assembly
                .resources
                .read::<HookFault>()
                .unwrap()
                .failed_task
                .expect("preparation failure was not injected");
            let control = assembly.resources.read::<Control>().unwrap();
            if !control
                .instance(target)
                .is_some_and(|item| item.state == UnitState::Dead)
            {
                continue;
            }
        }
        if stage != 0 && stage != observed {
            let roster = assembly
                .resources
                .read::<crate::system::control::identity::Roster>()
                .unwrap();
            let binding = crate::system::control::identity::binding(&roster, target).unwrap();
            let runtime = assembly
                .resources
                .read::<crate::system::publication::RuntimeNamespace>()
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
            .read::<Control>()
            .unwrap()
            .instances()
            .all(|item| item.team.is_none());
        let registered = assembly
            .resources
            .read::<crate::system::publication::RuntimeNamespace>()
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
        execution::room::park(core::time::Duration::from_millis(1)).unwrap();
    }
    let worker_id = worker.id();
    let peer_id = peer.id();
    worker.join();
    peer.join();
    {
        let roster = assembly
            .resources
            .read::<crate::system::control::identity::Roster>()
            .unwrap();
        roster.unbind(worker_id).unwrap();
        roster.unbind(peer_id).unwrap();
        let target = TaskId::new(signals.target.load(Ordering::Acquire));
        assert!(
            crate::system::control::identity::binding(&roster, target)
                .unwrap()
                .is_none()
        );
    }
    {
        let mut system = assembly.resources.write::<Control>().unwrap();
        system.fixture_detach_unit("login");
        system.fixture_detach_unit("terminal");
    }
    assembly.progress().unwrap();
    assert!(
        operator
            .tile(
                system_api::operator::path::Path::new("/idt/principal/anran/ref"),
                WAIT
            )
            .unwrap()
            .token(WAIT)
            .is_ok()
    );
    {
        let fault = assembly.resources.read::<HookFault>().unwrap();
        assert!(
            fault.waited && fault.retired_failed && fault.failed_task.is_some(),
            "hook failure coverage missing"
        );
    }
    assert_eq!(
        unit::heir_count(),
        heirs,
        "account-instance: runtime team leaked"
    );
    programs::debug::put(
        "account-instance: hook wait/failure rollback, foreign caller, expiry, start failure and owner death cleaned",
    );
}
