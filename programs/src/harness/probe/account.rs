//! Real IPC authorized construction and instance failures exercised against an isolated Control fixture.
use super::fixture::Fixture;
use crate::system::app::Fault as Fail;
use crate::system::control::{instance::hook, unit::Control, unit::table::State as UnitState};

use ::schedule::{Cursor, Progress, Schedule};
use account_client::Client;
use alloc::sync::Arc;
use core::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use env::{Permission, TaskId, Wait, unit};
use system_api::control;
use system_api::control::State;
use system_client::control::Face;

const WAIT: Wait = Wait::AtMost(2000);
static CREATOR: &[crate::unit::Construction] = &[crate::unit::Construction::unit("shell")];
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
    active: ::schedule::Res<crate::system::assembly::Jobs>,
    resources: ::schedule::Res<crate::system::assembly::namespace::NamespaceInstallations>,
) -> Result<Progress, &'static str> {
    if fault.prepare {
        let task = active.active_hook_task().ok_or("probe hook target")?;
        assert!(
            resources.prepared(task),
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
    active: ::schedule::Res<crate::system::assembly::Jobs>,
    control: ::schedule::Res<Control>,
) -> Result<Progress, &'static str> {
    let item = control
        .instances()
        .find(|item| Some(item.task) == active.active_hook_task())
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
    let children = crate::system::assembly::hooks::children().unwrap();
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
    terminal_entry: AtomicUsize,
    terminal_target: AtomicUsize,
    terminal_ready: AtomicUsize,
    inherited_ready: AtomicBool,
    inherited_done: AtomicBool,
    creator_request: AtomicBool,
    creator_ready: AtomicBool,
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
fn reference(context: (&system_client::operator::Face, TaskId), task: TaskId) -> system_api::identity::OperatorEntryRef {
    use system_api::identity::{Grant, IdentityName, IdentityObject, Reply, Wire};
    let (tree, authority) = context;
    let entry = ipc::session::establish::find(authority, Grant::Resolve.mark()).unwrap();
    let face = system_client::identity::Face::direct(authority, Grant::Resolve, entry).unwrap();
    let Reply::Binding(Some(binding)) = face.call(Wire::Resolve(task), WAIT).unwrap() else {
        panic!("account child binding")
    };
    let reference_face = system_client::identity::Face::discover(tree, authority, Grant::Reference, WAIT).unwrap();
    let Reply::Reference(Some(reference)) = reference_face
        .call(
            Wire::Reference(
                IdentityObject::Principal(binding.current.principal),
                IdentityName::new("anran").unwrap(),
            ),
            WAIT,
        )
        .unwrap()
    else {
        panic!("account entry association")
    };
    reference
}
pub(crate) fn acceptance(assembly: &mut Fixture, operator: &system_client::operator::Face) {
    let root = unit::self_id();
    let heirs = {
        let mut page = [0u64; 64];
        env::unit::scan(
            env::TeamId::new(0),
            env::VirtAddr::new(page.as_mut_ptr() as usize),
            page.len(),
        )
        .expect("Scan heirs")
    };
    let authority = assembly
        .resources
        .read::<crate::system::control::identity::Roster>()
        .unwrap()
        .authority()
        .unwrap();
    let account_task = assembly
        .resources
        .read::<Control>()
        .unwrap()
        .task("account")
        .unwrap();
    let signals = Arc::new(Signals {
        pings: AtomicUsize::new(0),
        stage: AtomicUsize::new(0),
        ack: AtomicUsize::new(0),
        target: AtomicUsize::new(0),
        peer_done: AtomicBool::new(false),
        done: AtomicBool::new(false),
        entry: AtomicUsize::new(0),
        terminal_entry: AtomicUsize::new(0),
        terminal_target: AtomicUsize::new(0),
        terminal_ready: AtomicUsize::new(0),
        inherited_ready: AtomicBool::new(false),
        inherited_done: AtomicBool::new(false),
        creator_request: AtomicBool::new(false),
        creator_ready: AtomicBool::new(false),
    });
    let s = signals.clone();
    let worker = execution::unit::task::spawn(move || {
        until(|| s.entry.load(Ordering::Acquire) != 0);
        let tree = system_client::operator::Face::of(
            ipc::session::Session::open(root, system_client::operator::BERTH, WAIT).unwrap(),
        );
        let client = Client::find(&tree, WAIT).unwrap();
        let entry = ipc::session::establish::find(account_task, account_api::ENTRY).unwrap();
        // Oversized input must be discarded before the valid requests below.
        resource::raw::Hole::from_raw(entry)
            .push(&[0; 65], WAIT)
            .unwrap();
        let lifecycle =
            Face::of(ipc::session::establish::find(root, control::ASK_MARK).unwrap()).unwrap();
        assert_eq!(
            client
                .create(
                    system_api::control::construction::CreateId { value: 1 },
                    "unknown",
                    WAIT
                )
                .err(),
            Some(control::Fail::Unknown)
        );
        let first = client
            .create(
                system_api::control::construction::CreateId { value: 2 },
                "anran",
                WAIT,
            )
            .unwrap();
        let account = reference((&tree, authority), first.task);
        system_client::identity::forward_resolve(
            first.startup.resolve,
            authority,
            root,
            first.task,
        )
        .unwrap();
        assert_eq!(
            client
                .create(
                    system_api::control::construction::CreateId { value: 2 },
                    "anran",
                    WAIT
                )
                .unwrap()
                .task,
            first.task,
            "a replayed Account CreateId returns the same constructor result"
        );
        assert_eq!(
            client
                .create(
                    system_api::control::construction::CreateId { value: 2 },
                    "other",
                    WAIT
                )
                .err(),
            Some(control::Fail::Conflict),
            "reusing an Account CreateId with different parameters conflicts"
        );
        let first_issuer = crate::user::login::startup::deliver(first.task, root)
            .expect("account-instance: issue first Shell system context");
        s.terminal_target.store(first.task.get(), Ordering::Release);
        until(|| s.terminal_ready.load(Ordering::Acquire) == first.task.get());
        lifecycle.instance(first.task).embark(WAIT).unwrap();
        let second = client
            .create(
                system_api::control::construction::CreateId { value: 3 },
                "anran",
                WAIT,
            )
            .unwrap();
        assert_ne!(first.task, second.task);
        assert_ne!(first.team, second.team);
        assert_eq!(reference((&tree, authority), second.task), account);
        system_client::identity::forward_resolve(
            second.startup.resolve,
            authority,
            root,
            second.task,
        )
        .unwrap();
        let second_issuer = crate::user::login::startup::deliver(second.task, root)
            .expect("account-instance: issue second Shell system context");
        s.terminal_target.store(second.task.get(), Ordering::Release);
        until(|| s.terminal_ready.load(Ordering::Acquire) == second.task.get());
        lifecycle.instance(second.task).embark(WAIT).unwrap();
        lifecycle.instance(second.task).ruin(WAIT).unwrap();
        drop(second_issuer);
        assert_eq!(
            lifecycle.instance(second.task).state(WAIT).unwrap(),
            State::Dead
        );
        assert_eq!(
            lifecycle.instance(first.task).state(WAIT).unwrap(),
            State::Ready
        );
        announce(&s, 1, first.task);
        until(|| s.peer_done.load(Ordering::Acquire));
        lifecycle.instance(first.task).ruin(WAIT).unwrap();
        drop(first_issuer);
        assert_eq!(
            lifecycle.instance(first.task).state(WAIT).unwrap(),
            State::Dead
        );
        let expired = client
            .create(
                system_api::control::construction::CreateId { value: 4 },
                "anran",
                WAIT,
            )
            .unwrap();
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
        let failed = client
            .create(
                system_api::control::construction::CreateId { value: 5 },
                "anran",
                WAIT,
            )
            .unwrap();
        announce(&s, 3, failed.task);
        assert!(matches!(
            lifecycle.instance(failed.task).embark(WAIT),
            Err(control::Fail::NotReady)
        ));
        closed(&lifecycle, failed.task);
        announce(&s, 13, failed.task);
        lifecycle.instance(failed.task).ruin(WAIT).unwrap();

        announce(&s, 5, TaskId::new(0));
        assert_eq!(
            client
                .create(
                    system_api::control::construction::CreateId { value: 6 },
                    "anran",
                    WAIT
                )
                .err(),
            Some(control::Fail::NotReady)
        );
        announce(&s, 15, TaskId::new(0));
        let abandoned = client
            .create(
                system_api::control::construction::CreateId { value: 7 },
                "anran",
                WAIT,
            )
            .unwrap();
        announce(&s, 4, abandoned.task);
        s.done.store(true, Ordering::Release);
        // Exiting with a unclaimed instance exercises automatic owner-death cleanup.
    });
    let s = signals.clone();
    let entry = operator
        .tile(account_api::DIR, WAIT)
        .unwrap()
        .token(WAIT)
        .unwrap();
    assert!(
        matches!(resource::raw::reserve(entry), Ok((_, owner, mark)) if owner == account_task && mark == account_api::ENTRY)
    );
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
        let terminal = env::pie::unseal(env::UnsealArgs::hole(terminal_api::marks::ENTRY)).unwrap();
        let input = env::pie::unseal(env::UnsealArgs::hole(terminal_api::frame::INPUT)).unwrap();
        let output = env::pie::unseal(env::UnsealArgs::hole(terminal_api::frame::OUTPUT)).unwrap();
        let control = env::pie::unseal(env::UnsealArgs::hole(terminal_api::frame::CONTROL)).unwrap();
        let delivered = resource::port::ship(
            terminal,
            root,
            env::Access::FETCH | env::Access::STORE,
            env::Policy::VEST,
        )
        .unwrap()
        .seed();
        p.terminal_entry.store(delivered.get(), Ordering::Release);
        let mut injected = 0usize;
        let mut terminals = alloc::vec::Vec::new();
        let drain = |terminals: &[TaskId]| {
            let mut bytes = [0; terminal_api::frame::MAX];
            while let Ok((n, from)) = resource::raw::Hole::from_raw(output).pull(&mut bytes, Wait::POLL) {
                assert!(n > 0 && terminals.contains(&from), "fixture Terminal source");
            }
        };
        while p.stage.load(Ordering::Acquire) != 1 || token.load(Ordering::Acquire) == 0 {
            let target = p.terminal_target.load(Ordering::Acquire);
            if target != 0 && target != injected {
                let target = TaskId::new(target);
                for (endpoint, permission, mark) in [
                    (input, Permission::FETCH, terminal_api::frame::INPUT),
                    (output, Permission::STORE, terminal_api::frame::OUTPUT),
                    (control, Permission::FETCH, terminal_api::frame::CONTROL),
                ] {
                    env::pie::accord(endpoint, target, permission, mark).unwrap();
                }
                terminals.push(target);
                injected = target.get();
                p.terminal_ready.store(injected, Ordering::Release);
            }
            drain(&terminals);
            p.pings.fetch_add(1, Ordering::Release);
            execution::room::park(core::time::Duration::from_millis(1)).unwrap();
        }
        until(|| p.stage.load(Ordering::Acquire) == 1 && token.load(Ordering::Acquire) != 0);
        let tree = system_client::operator::Face::of(
            ipc::session::Session::open(root, system_client::operator::BERTH, WAIT).unwrap(),
        );
        let client = Client::find(&tree, WAIT).unwrap();
        let lifecycle =
            Face::of(ipc::session::establish::find(root, control::ASK_MARK).unwrap()).unwrap();
        let target = TaskId::new(p.target.load(Ordering::Acquire));
        {
            use system_api::control::construction as api;
            let entry = ipc::session::establish::find(root, api::ENTRY).unwrap();
            let sender =
                ipc::rpc::request::Sender::<api::Call>::from_raw(entry, api::Call::BACK).unwrap();
            let forged = system_api::identity::Subject::new(
                system_api::identity::PrincipalId::root(authority),
                &[],
            )
            .unwrap();
            let said = sender
                .call(ipc::time::Deadline::new(WAIT), |back| api::Request {
                    op: api::BUILD,
                    id: api::CreateId { value: 1 },
                    input: api::InputOption(Some(api::Input::Unit(api::UnitRef {
                        name: "shell".into(),
                        stack: 0,
                        count: 0,
                        args: [0; system_api::loader::MAX_ARGS],
                    }))),
                    owner: root,
                    subject: forged,
                    back,
                })
                .unwrap();
            assert_eq!(
                said.status,
                system_api::control::frame::DENIED,
                "foreign creator selected another Subject"
            );
            p.creator_request.store(true, Ordering::Release);
            until(|| p.creator_ready.load(Ordering::Acquire));
            let own_entry = resource::raw::pies()
                .find(|info| {
                    info.alive
                        && info.owner == root
                        && info.mark == api::ENTRY
                        && info.token != entry
                })
                .unwrap()
                .token;
            let sender =
                ipc::rpc::request::Sender::<api::Call>::from_raw(own_entry, api::Call::BACK)
                    .unwrap();
            let said = sender
                .call(ipc::time::Deadline::new(WAIT), |back| api::Request {
                    op: api::BUILD,
                    id: api::CreateId { value: 2 },
                    input: api::InputOption(Some(api::Input::Unit(api::UnitRef {
                        name: "shell".into(),
                        stack: 0,
                        count: 0,
                        args: [0; system_api::loader::MAX_ARGS],
                    }))),
                    owner: root,
                    subject: forged,
                    back,
                })
                .unwrap();
            assert_eq!(
                said.status,
                system_api::control::frame::DENIED,
                "authorized creator escaped its identity subtree"
            );
        }

        assert!(matches!(
            client.create(
                system_api::control::construction::CreateId { value: 1 },
                "anran",
                WAIT
            ),
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
            drain(&terminals);
            p.pings.fetch_add(1, Ordering::Release);
            execution::room::park(core::time::Duration::from_millis(1)).unwrap();
        }
    });
    let inherited_signals = signals.clone();
    let inherited = execution::unit::task::spawn(move || {
        until(|| inherited_signals.inherited_ready.load(Ordering::Acquire));
        let tree = system_client::operator::Face::of(
            ipc::session::Session::open(root, system_client::operator::BERTH, WAIT).unwrap(),
        );
        let client = Client::find(&tree, WAIT).unwrap();
        assert_eq!(
            client
                .create(
                    system_api::control::construction::CreateId { value: 1 },
                    "anran",
                    WAIT
                )
                .err(),
            Some(control::Fail::Denied),
            "a task inheriting Login's principal impersonated the named Login deployment"
        );
        inherited_signals
            .inherited_done
            .store(true, Ordering::Release);
    });
    env::pie::accord(
        instance_entry,
        peer.id(),
        Permission::FETCH | Permission::STORE,
        control::ASK_MARK,
    )
    .unwrap();
    let construction = assembly
        .resources
        .read::<crate::system::control::Construction>()
        .unwrap()
        .entries()
        .next()
        .unwrap();
    resource::port::ship(
        construction,
        peer.id(),
        env::Access::STORE,
        env::Policy::NONE,
    )
    .unwrap();
    peer_token.store(1, Ordering::Release);
    {
        let mut system = assembly.resources.write::<Control>().unwrap();
        system
            .fixture_attach_unit(&crate::unit::login::PROGRAM, worker.id())
            .unwrap();
        system.register_internal(peer.id()).unwrap();
        system.register_internal(inherited.id()).unwrap();
    }
    let login_subject = {
        let mut roster = assembly
            .resources
            .write::<crate::system::control::identity::Roster>()
            .unwrap();
        roster.authorize(worker.id()).unwrap();
        roster.authorize(peer.id()).unwrap();
        roster.inherit(inherited.id(), worker.id()).unwrap();
        let login = crate::system::control::identity::binding(&roster, worker.id())
            .unwrap()
            .unwrap()
            .current;
        let outsider = crate::system::control::identity::binding(&roster, peer.id())
            .unwrap()
            .unwrap()
            .current;
        assert_ne!(login.principal, outsider.principal);
        let child = crate::system::control::identity::binding(&roster, inherited.id())
            .unwrap()
            .unwrap()
            .current;
        assert_eq!(child.principal, login.principal);
        login
    };
    for task in [worker.id(), peer.id(), inherited.id()] {
        crate::system::operator::connect_task(&assembly.resources, task).unwrap();
    }
    until(|| signals.terminal_entry.load(Ordering::Acquire) != 0);
    let terminal = env::PieToken::from_bytes(
        &(signals.terminal_entry.load(Ordering::Acquire) as u64).to_le_bytes(),
    )
    .unwrap();
    assert!(
        matches!(resource::raw::reserve(terminal), Ok((vestor, owner, mark)) if vestor == peer.id() && owner == peer.id() && mark == terminal_api::marks::ENTRY)
    );
    let terminal_mounts = {
        let start = assembly.resources.read::<crate::system::app::operator::Layout>().unwrap()
            .as_ref().unwrap().reserve_operations(3).unwrap().0;
        let info = assembly
            .resources
            .read::<crate::system::operator::management::Tree>()
            .unwrap()
            .root_entry()
            .unwrap();
        let capability =
            system_client::operator::Capability::direct(info.entry.operator, info.entrance)
                .unwrap();
        let services = capability
            .part(
                info.entry,
                "svc",
                system_api::operator::OperationId(start),
                Wait::AtMost(3000),
            )
            .unwrap()
            .entry;
        let pane = capability
            .part(
                services,
                "terminal",
                system_api::operator::OperationId(start + 1),
                Wait::AtMost(3000),
            )
            .unwrap()
            .entry;
        let view = system_api::operator::ResourceView::new(env::Access::FETCH | env::Access::STORE)
            .unwrap();
        let binding = capability
            .bind(
                pane,
                "attach",
                terminal,
                view,
                system_api::operator::Permit::Bound,
                system_api::operator::OperationId(start + 2),
                Wait::AtMost(3000),
            )
            .unwrap()
            .binding;
        (capability, pane, binding)
    };
    assembly
        .resources
        .insert(HookFault {
            signals: Some(signals.clone()),
            ..HookFault::default()
        })
        .unwrap();
    let mut schedule = Schedule::new();
    schedule
        .add_plan("control.poll", 0u8, crate::system::control::poll().unwrap())
        .unwrap();
    schedule
        .add_system(
            "construction.receive",
            0u8,
            crate::system::control::receive_construction,
        )
        .unwrap();
    schedule
        .add_system(
            "construction.admit",
            1,
            crate::system::control::admit_construction,
        )
        .unwrap();
    schedule
        .add_plan("launch", 2, crate::system::launch::frame().unwrap())
        .unwrap();
    schedule
        .add_plan(
            "assembly.jobs",
            3,
            crate::system::assembly::frame().unwrap(),
        )
        .unwrap();
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
        .add_system("launch.completed", 10, crate::system::assembly::completed)
        .unwrap();
    let mut plan = schedule.build().unwrap();
    plan.prepare(&assembly.resources);
    let mut cursor = Cursor::default();
    s.entry.store(1, Ordering::Release);
    signals.inherited_ready.store(true, Ordering::Release);
    let deadline = env::chrono::clock() + 15_000_000_000;
    let mut observed = 0;
    let mut user = None;
    loop {
        if signals.creator_request.load(Ordering::Acquire)
            && !signals.creator_ready.load(Ordering::Acquire)
        {
            assembly
                .resources
                .write::<crate::system::control::Construction>()
                .unwrap()
                .grant(peer.id(), CREATOR)
                .unwrap();
            signals.creator_ready.store(true, Ordering::Release);
        }
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
            let mut roster = assembly
                .resources
                .write::<crate::system::control::identity::Roster>()
                .unwrap();
            let binding = crate::system::control::identity::binding(&roster, target).unwrap();
            let runtime = assembly
                .resources
                .read::<crate::system::assembly::namespace::NamespaceInstallations>()
                .unwrap()
                .prepared(target);
            if stage < 10 {
                let subject = binding
                    .expect("account-instance: Held task has no identity")
                    .current;
                assert_ne!(subject.principal, login_subject.principal);
                assert!(
                    runtime,
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
                assert!(!runtime, "account-instance: Dead task still registered");
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
            .read::<crate::system::assembly::namespace::NamespaceInstallations>()
            .unwrap()
            .prepared(target);
        if signals.done.load(Ordering::Acquire)
            && signals.inherited_done.load(Ordering::Acquire)
            && unit::join_task(worker.id(), Wait::POLL).unwrap_or(true)
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
    let inherited_id = inherited.id();
    worker.join();
    peer.join();
    inherited.join();
    terminal_mounts
        .0
        .unbind(terminal_mounts.2, Wait::AtMost(3000))
        .unwrap();
    terminal_mounts
        .0
        .unpart(terminal_mounts.1, Wait::AtMost(3000))
        .unwrap();
    let _ = env::pie::release(terminal, env::ReleaseMode::Revoke);
    assembly
        .resources
        .write::<Control>()
        .unwrap()
        .fixture_detach_unit("login");
    {
        let mut roster = assembly
            .resources
            .write::<crate::system::control::identity::Roster>()
            .unwrap();
        roster.unbind(worker_id).unwrap();
        roster.unbind(peer_id).unwrap();
        roster.unbind(inherited_id).unwrap();
        let target = TaskId::new(signals.target.load(Ordering::Acquire));
        assert!(
            crate::system::control::identity::binding(&roster, target)
                .unwrap()
                .is_none()
        );
    }
    assembly.progress().unwrap();
    {
        let fault = assembly.resources.read::<HookFault>().unwrap();
        assert!(
            fault.waited && fault.retired_failed && fault.failed_task.is_some(),
            "hook failure coverage missing"
        );
    }
    assert_eq!(
        {
            let mut page = [0u64; 64];
            env::unit::scan(
                env::TeamId::new(0),
                env::VirtAddr::new(page.as_mut_ptr() as usize),
                page.len(),
            )
            .expect("Scan heirs")
        },
        heirs,
        "account-instance: runtime team leaked"
    );
    programs::debug::put(
        "account-instance: hook wait/failure rollback, foreign caller, expiry, start failure and owner death cleaned",
    );
}
