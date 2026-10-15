use super::fixture::Fixture;
use crate::system::loader::{Image, Loader};
use ::schedule::{Cursor, Progress, Schedule};
use alloc::sync::Arc;
use core::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use env::{Permission, TaskId, Wait};
use ipc::session::establish;
use system_client::loader as call;
use system_client::operator::Face as Operator;

const WAIT: Wait = Wait::AtMost(2000);
static CONSTRUCTION: &[crate::unit::Construction] = &[crate::unit::Construction::image()];

pub(crate) fn acceptance(assembly: &mut Fixture, operator: &Operator) {
    let service = operator
        .resolve(system_api::namespace::SERVICE, WAIT)
        .unwrap();
    let road = system_api::operator::Path::new(system_api::loader::DIR)
        .try_join(system_api::loader::Grant::Claim.name())
        .unwrap();
    assert!(
        operator
            .tile_from(service, &road, WAIT)
            .unwrap()
            .token(WAIT)
            .is_ok(),
        "loader: operator entry missing"
    );
    shared_prefix_retirement(assembly, operator);
    let root = env::unit::self_id();
    let authority = assembly
        .resources
        .read::<crate::system::control::identity::Roster>()
        .unwrap()
        .authority()
        .unwrap();
    let heirs = {
        let mut page = [0u64; 64];
        env::unit::scan(
            env::TeamId::new(0),
            env::VirtAddr::new(page.as_mut_ptr() as usize),
            page.len(),
        )
        .expect("Scan heirs")
    };
    let ready = Arc::new(AtomicBool::new(false));
    let target = Arc::new(AtomicUsize::new(0));
    let denied = Arc::new(AtomicBool::new(false));
    let done = Arc::new(AtomicBool::new(false));
    let r = ready.clone();
    let t = target.clone();
    let d = denied.clone();
    let peer = execution::unit::task::spawn(move || {
        until(|| r.load(Ordering::Acquire) && t.load(Ordering::Acquire) != 0);
        let face = system_client::control::Face::of(
            establish::find(root, system_api::control::Grant::State.mark()).unwrap(),
        )
        .unwrap();
        assert_eq!(
            face.instance(TaskId::new(t.load(Ordering::Acquire)))
                .state(WAIT),
            Err(system_api::control::Fail::Denied)
        );
        d.store(true, Ordering::Release);
    });
    let r = ready.clone();
    let worker_done = done.clone();
    let worker_target = target.clone();
    let worker = execution::unit::task::spawn(move || {
        until(|| r.load(Ordering::Acquire));
        exercise((root, authority), &worker_target, &denied);
        worker_done.store(true, Ordering::Release);
    });
    assembly
        .resources
        .write::<crate::system::control::Construction>()
        .unwrap()
        .grant(worker.id(), CONSTRUCTION)
        .unwrap();
    let entry = crate::system::loader::entry(&assembly.resources).unwrap();
    env::pie::accord(
        entry,
        worker.id(),
        Permission::STORE,
        system_api::loader::Grant::Claim.mark(),
    )
    .unwrap();
    for grant in [
        system_api::control::Grant::State,
        system_api::control::Grant::Embark,
        system_api::control::Grant::Debark,
        system_api::control::Grant::Ruin,
    ] {
        let entry = crate::system::control::entry(&assembly.resources, grant).unwrap();
        env::pie::accord(entry, worker.id(), Permission::STORE, grant.mark()).unwrap();
        if grant == system_api::control::Grant::State {
            env::pie::accord(entry, peer.id(), Permission::STORE, grant.mark()).unwrap();
        }
    }
    {
        let mut roster = assembly
            .resources
            .write::<crate::system::control::identity::Roster>()
            .unwrap();
        roster.inherit(worker.id(), root).unwrap();
        roster.inherit(peer.id(), root).unwrap();
    }
    {
        let mut control = assembly
            .resources
            .write::<crate::system::control::unit::Control>()
            .unwrap();
        control.register_internal(worker.id()).unwrap();
        control.register_internal(peer.id()).unwrap();
    }
    let mut schedule = Schedule::new();
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
            0u8,
            crate::system::control::admit_construction,
        )
        .unwrap();
    schedule
        .add_plan("launch", 0u8, crate::system::launch::frame().unwrap())
        .unwrap();
    schedule
        .add_plan(
            "assembly.jobs",
            1u8,
            crate::system::assembly::frame().unwrap(),
        )
        .unwrap();
    schedule
        .add_system("receive", 2, crate::system::control::receive)
        .unwrap();
    schedule
        .add_system("instances", 3, crate::system::control::answer_instances)
        .unwrap();
    schedule
        .add_system("reap", 4, crate::system::control::instance::schedule::reap)
        .unwrap();
    schedule
        .add_plan(
            "instance.hooks",
            5,
            crate::system::assembly::hooks::instance().unwrap(),
        )
        .unwrap();
    schedule
        .add_system("launch.completed", 7, crate::system::assembly::completed)
        .unwrap();
    let mut plan = schedule.build().unwrap();
    plan.prepare(&assembly.resources);
    let mut cursor = Cursor::default();
    ready.store(true, Ordering::Release);
    let deadline = env::chrono::clock() + 12_000_000_000;
    let mut registered = false;
    loop {
        assembly.progress().unwrap();
        if plan.advance(&mut cursor, &assembly.resources).unwrap() == Progress::Done {
            cursor.reset();
        }
        let task = TaskId::new(target.load(Ordering::Acquire));
        if task.get() != 0
            && assembly
                .resources
                .read::<crate::system::assembly::namespace::NamespaceInstallations>()
                .unwrap()
                .prepared(task)
        {
            registered = true;
        }
        let clean = assembly
            .resources
            .read::<crate::system::control::unit::Control>()
            .unwrap()
            .instances()
            .all(|item| item.team.is_none());
        if done.load(Ordering::Acquire) && clean {
            break;
        }
        assert!(
            env::chrono::clock() < deadline,
            "loader: IPC fixture timed out"
        );
        execution::room::park(core::time::Duration::from_millis(1)).unwrap();
    }
    worker.join();
    peer.join();
    assert!(
        registered,
        "loader: instance missing from runtime resources"
    );
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
        "loader: runtime team leaked"
    );
    cache_clear();
    programs::debug::put(
        "loader: operator entry, submitted ELF, instances, owner checks and unclaimed cleanup passed",
    );
}
/// Exercise placement cleanup against the production Operator RPC and receipts.
fn shared_prefix_retirement(assembly: &Fixture, operator: &Operator) {
    use crate::system::app::{
        namespace::{Preparation, Prepare, ResourceInput},
        operator::Layout,
    };
    use system_api::operator::PathBuf;

    let layouts = assembly.resources.read::<Layout>().unwrap();
    let system = layouts.as_ref().expect("loader: namespace layout missing");
    let unit = system.resolve(system_api::namespace::UNIT, WAIT).unwrap();
    let mut first = Preparation::new(
        Prepare::resources(ResourceInput {
            base: unit,
            path: PathBuf::try_new("namespace-cleanup/first").unwrap(),
            retain_prefix: 1,
            bindings: alloc::vec::Vec::new(),
            wait: WAIT,
        })
        .unwrap(),
    );
    let mut second = Preparation::new(
        Prepare::resources(ResourceInput {
            base: unit,
            path: PathBuf::try_new("namespace-cleanup/second").unwrap(),
            retain_prefix: 1,
            bindings: alloc::vec::Vec::new(),
            wait: WAIT,
        })
        .unwrap(),
    );
    system.prepare_resources(&mut first).unwrap();
    system.prepare_resources(&mut second).unwrap();
    assert!(system.retire_resources(&mut first).unwrap());
    assert_eq!(
        operator.seek_from(
            unit,
            system_api::operator::Path::new("namespace-cleanup/first"),
            WAIT
        ),
        Err(system_api::operator::Fail::Unknown),
        "retired placement leaf must disappear"
    );
    let parent = operator
        .seek_from(
            unit,
            system_api::operator::Path::new("namespace-cleanup"),
            WAIT,
        )
        .unwrap();
    assert!(
        operator
            .seek_from(
                unit,
                system_api::operator::Path::new("namespace-cleanup/second"),
                WAIT
            )
            .is_ok(),
        "retiring one task must preserve its live sibling"
    );
    assert!(system.retire_resources(&mut second).unwrap());
    assert_eq!(
        operator.seek_from(
            unit,
            system_api::operator::Path::new("namespace-cleanup/second"),
            WAIT
        ),
        Err(system_api::operator::Fail::Unknown),
        "retired sibling leaf must disappear"
    );
    assert_eq!(
        operator.seek_from(
            unit,
            system_api::operator::Path::new("namespace-cleanup"),
            WAIT
        ),
        Ok(parent),
        "the shared placement parent belongs to the System lifecycle"
    );
    system.capability().unpart(parent, WAIT).unwrap();
    programs::debug::put("loader: namespace shared-parent/sibling retirement passed");
}

fn until(mut ready: impl FnMut() -> bool) {
    let deadline = env::chrono::clock() + 5_000_000_000;
    while !ready() {
        assert!(env::chrono::clock() < deadline, "loader: waiting timed out");
        execution::room::park(core::time::Duration::from_millis(1)).unwrap();
    }
}
fn exercise(context: (TaskId, TaskId), target: &AtomicUsize, denied: &AtomicBool) {
    let (root, authority) = context;
    let entry = establish::find(root, system_api::control::construction::ENTRY).unwrap();
    let loader =
        call::Face::of(establish::find(root, system_api::loader::Grant::Claim.mark()).unwrap())
            .unwrap();
    let resolve = system_client::identity::Face::direct(
        authority,
        system_api::identity::Grant::Resolve,
        establish::find(authority, system_api::identity::Grant::Resolve.mark()).unwrap(),
    )
    .unwrap();
    let subject = match resolve
        .call(
            system_api::identity::Wire::Resolve(env::unit::self_id()),
            WAIT,
        )
        .unwrap()
    {
        system_api::identity::Reply::Binding(Some(binding)) => binding.current,
        _ => panic!("loader: requester identity missing"),
    };
    let constructor =
        ipc::rpc::request::Sender::<system_api::control::construction::Call>::from_raw(
            entry,
            system_api::control::construction::BACK,
        )
        .unwrap();
    let next_id = core::cell::Cell::new(0u64);
    assert!(
        env::unit::build(env::ProgramKind::User, env::unit::self_id()).is_err(),
        "loader: caller received Build authority"
    );
    let image = ::resource::raw::pole(8192, true).unwrap();
    let bytes = image_bytes();
    let (at, size) = ::resource::raw::open(image).unwrap();
    // SAFETY: the locally owned writable Pole covers the complete test ELF.
    unsafe {
        core::ptr::copy_nonoverlapping(bytes.as_ptr(), at as *mut u8, bytes.len());
    }
    let build = |offset: usize, len: usize, args: &[u64]| {
        let id = next_id.get() + 1;
        next_id.set(id);
        let said = create(&constructor, image, offset, len, args, subject, id);
        if said.status != wire::OK {
            return Err(said.status);
        }
        system_client::identity::forward_resolve(said.startup.resolve, authority, root, said.task)
            .unwrap();
        Ok(loader.claim(said.task, WAIT).unwrap())
    };
    let first = build(0, bytes.len(), &[7, 11]).unwrap();
    let second = build(0, bytes.len(), &[7, 11]).unwrap();
    assert_ne!(first.task, second.task);
    assert_ne!(first.team, second.team);
    target.store(first.task.get(), Ordering::Release);
    until(|| denied.load(Ordering::Acquire));
    let face = |grant: system_api::control::Grant| {
        system_client::control::Face::of(establish::find(root, grant.mark()).unwrap()).unwrap()
    };
    let state = face(system_api::control::Grant::State);
    assert_eq!(
        state.instance(first.task).state(WAIT).unwrap(),
        system_api::control::State::Debarked
    );
    assert!(env::unit::join_task(first.task, Wait::POLL).is_ok_and(|done| !done));
    face(system_api::control::Grant::Embark)
        .instance(first.task)
        .embark(WAIT)
        .unwrap();
    assert_eq!(
        state.instance(first.task).state(WAIT).unwrap(),
        system_api::control::State::Ready
    );
    until(|| {
        face(system_api::control::Grant::Debark)
            .instance(first.task)
            .debark(WAIT)
            .is_ok()
    });
    assert_eq!(
        state.instance(first.task).state(WAIT).unwrap(),
        system_api::control::State::Debarked
    );
    face(system_api::control::Grant::Embark)
        .instance(first.task)
        .embark(WAIT)
        .unwrap();
    face(system_api::control::Grant::Ruin)
        .instance(first.task)
        .ruin(WAIT)
        .unwrap();
    until(|| state.instance(first.task).state(WAIT) == Ok(system_api::control::State::Dead));
    face(system_api::control::Grant::Ruin)
        .instance(first.task)
        .ruin(WAIT)
        .unwrap();
    assert_eq!(
        state.instance(second.task).state(WAIT).unwrap(),
        system_api::control::State::Debarked
    );
    assert!(matches!(
        build(size, 1, &[]),
        Err(status) if status == system_api::control::frame::fail_to_code(Some(system_api::control::Fail::BadImage))
    ));
    // SAFETY: the caller retains write access while the service uses an independent snapshot.
    unsafe {
        *(at as *mut u8) = 0;
    }
    assert!(matches!(
        build(0, bytes.len(), &[]),
        Err(status) if status == system_api::control::frame::fail_to_code(Some(system_api::control::Fail::BadImage))
    ));
    assert_eq!(
        state.instance(second.task).state(WAIT).unwrap(),
        system_api::control::State::Debarked
    );
    unsafe {
        *(at as *mut u8) = 0x7f;
    }

    let id = next_id.get() + 1;
    let said = create(&constructor, image, 0, bytes.len(), &[], subject, id);
    assert_eq!(said.status, wire::OK);
    // Leave the result unclaimed while keeping its requester alive.
    until(|| state.instance(said.task).state(WAIT) == Ok(system_api::control::State::Dead));
    env::pie::shut(image).unwrap();
    env::pie::release(image, env::ReleaseMode::Revoke).unwrap();
    // The second, claimed Held task is reclaimed when this requester exits.
}

fn create(
    constructor: &ipc::rpc::request::Sender<system_api::control::construction::Call>,
    image: env::PieToken,
    offset: usize,
    len: usize,
    args: &[u64],
    subject: system_api::identity::Subject,
    id: u64,
) -> system_api::loader::Said {
    use system_api::control::construction as api;
    let peer = constructor.peer();
    let loan = env::pie::accord(image, peer, Permission::FETCH, system_api::loader::IMAGE).unwrap();
    let mut arguments = [0; system_api::loader::MAX_ARGS];
    arguments[..args.len()].copy_from_slice(args);
    let said = constructor
        .call(ipc::time::Deadline::new(WAIT), |back| api::Request {
            op: api::BUILD,
            id: api::CreateId { value: id },
            input: api::InputOption(Some(api::Input::Image(api::Image {
                image: loan,
                offset: offset as u64,
                len: len as u64,
                stack: 0,
                count: args.len() as u8,
                args: arguments,
            }))),
            owner: env::unit::self_id(),
            subject,
            back,
        })
        .unwrap();
    assert!(
        matches!(env::pie::revoke(peer, loan),
        Err(error) if error.source == env::PieFail::Denied),
        "loader: constructor did not release its image loan"
    );
    said
}

fn cache_clear() {
    let bytes = image_bytes();
    let before = ::resource::raw::table_size();
    let mut loader = Loader::new();
    let image = loader
        .build(Image {
            bytes: &bytes,
            kind: env::ProgramKind::User,
        })
        .expect("loader: cache fixture image");
    drop(image);
    assert_eq!(
        loader.cached_entries(),
        1,
        "loader: cache fixture not retained"
    );
    let cached = ::resource::raw::table_size();
    assert!(
        cached > before,
        "loader: cached pole missing from resource table"
    );
    loader.clear_images();
    assert_eq!(
        loader.cached_entries(),
        0,
        "loader: clear retained cache entry"
    );
    assert!(
        ::resource::raw::table_size() < cached,
        "loader: cache clear did not release its mapped source"
    );
}

fn image_bytes() -> alloc::vec::Vec<u8> {
    const IMAGE_BASE: u64 = 0x1_0000;
    let mut bytes = alloc::vec![0; 4100];
    bytes[..7].copy_from_slice(&[0x7f, b'E', b'L', b'F', 2, 1, 1]);
    for (at, value) in [(16, 2u16), (18, 243), (52, 64), (54, 56), (56, 1)] {
        bytes[at..at + 2].copy_from_slice(&value.to_le_bytes());
    }
    bytes[20..24].copy_from_slice(&1u32.to_le_bytes());
    bytes[24..32].copy_from_slice(&IMAGE_BASE.to_le_bytes());
    bytes[32..40].copy_from_slice(&64u64.to_le_bytes());
    bytes[64..68].copy_from_slice(&1u32.to_le_bytes());
    bytes[68..72].copy_from_slice(&5u32.to_le_bytes());
    for (offset, value) in [(8, 4096u64), (16, IMAGE_BASE), (32, 4), (40, 4), (48, 4096)] {
        bytes[64 + offset..72 + offset].copy_from_slice(&value.to_le_bytes());
    }
    bytes[4096..].copy_from_slice(&[0x6f, 0, 0, 0]);
    bytes
}
