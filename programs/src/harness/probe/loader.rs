use super::fixture::Fixture;
use crate::system::loader::{Image, Loader};
use ::resource::raw::Hole;
use ::schedule::{Cursor, Progress, Schedule};
use alloc::sync::Arc;
use core::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use env::{Mark, Permission, TaskId, Wait};
use ipc::session::establish;
use system_client::loader as call;
use system_client::operator::Face as Operator;

const WAIT: Wait = Wait::AtMost(2000);

pub(crate) fn acceptance(assembly: &mut Fixture, operator: &Operator) {
    let road = system_api::operator::Path::new(system_api::loader::DIR)
        .try_join(system_api::loader::Grant::Build.name())
        .unwrap();
    assert!(
        operator.tile(&road, WAIT).unwrap().token(WAIT).is_ok(),
        "loader: operator entry missing"
    );
    let root = env::unit::self_id();
    let heirs = env::unit::heir_count();
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
        exercise(root, &worker_target, &denied);
        worker_done.store(true, Ordering::Release);
    });
    let entry = crate::system::loader::entry(&assembly.resources).unwrap();
    env::pie::accord(entry, worker.id(), Permission::STORE, Mark::NONE).unwrap();
    for grant in [
        system_api::control::Grant::State,
        system_api::control::Grant::Embark,
        system_api::control::Grant::Debark,
        system_api::control::Grant::Ruin,
    ] {
        let entry = crate::system::control::entry(&assembly.resources, grant).unwrap();
        env::pie::accord(entry, worker.id(), Permission::STORE, Mark::NONE).unwrap();
        if grant == system_api::control::Grant::State {
            env::pie::accord(entry, peer.id(), Permission::STORE, Mark::NONE).unwrap();
        }
    }
    {
        let roster = assembly
            .resources
            .read::<crate::system::control::identity::Roster>()
            .unwrap();
        roster.inherit(worker.id(), root).unwrap();
        roster.inherit(peer.id(), root).unwrap();
    }
    let mut schedule = Schedule::new();
    schedule
        .add_plan("loader", 0u8, crate::system::loader::frame().unwrap())
        .unwrap();
    schedule
        .add_system("launch.register", 1, crate::system::launch::register)
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
            crate::system::launch::hooks::instance().unwrap(),
        )
        .unwrap();
    schedule
        .add_system("launch.completed", 6, crate::system::launch::completed)
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
                .read::<crate::system::publication::RuntimeNamespace>()
                .unwrap()
                .runtime_road(task)
                .is_some()
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
        env::unit::heir_count(),
        heirs,
        "loader: runtime team leaked"
    );
    cache_clear();
    programs::debug::put(
        "loader: operator entry, submitted ELF, instances, owner checks and unclaimed cleanup passed",
    );
}
fn until(mut ready: impl FnMut() -> bool) {
    let deadline = env::chrono::clock() + 5_000_000_000;
    while !ready() {
        assert!(env::chrono::clock() < deadline, "loader: waiting timed out");
        execution::room::park(core::time::Duration::from_millis(1)).unwrap();
    }
}
fn exercise(root: TaskId, target: &AtomicUsize, denied: &AtomicBool) {
    use env::wire::Span;
    use wire::Message;
    let entry = establish::find(root, system_api::loader::Grant::Build.mark()).unwrap();
    let loader = call::Face::of(entry).unwrap();
    assert!(
        env::unit::build(env::ProgramKind::User).is_err(),
        "loader: caller received Build authority"
    );
    let image = env::pie::unseal_pole(8192, true).unwrap();
    let bytes = image_bytes();
    let (at, size) = ::resource::raw::open(image).unwrap();
    // SAFETY: the locally owned writable Pole covers the complete test ELF.
    unsafe {
        core::ptr::copy_nonoverlapping(bytes.as_ptr(), at as *mut u8, bytes.len());
    }
    let build = || {
        loader
            .build(image, 0, bytes.len(), &[7, 11], 0, WAIT)
            .unwrap()
    };
    let first = build();
    let second = build();
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
    assert!(
        env::unit::join(first.task, Wait::POLL).is_err_and(|e| e.source == env::UnitFail::Denied)
    );
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
    assert_eq!(
        state.instance(second.task).state(WAIT).unwrap(),
        system_api::control::State::Debarked
    );
    assert!(matches!(
        loader.build(image, size, 1, &[], 0, WAIT),
        Err(system_api::loader::Fail::BadImage)
    ));
    // SAFETY: the caller retains write access while the service uses an independent snapshot.
    unsafe {
        *(at as *mut u8) = 0;
    }
    assert!(matches!(
        loader.build(image, 0, bytes.len(), &[], 0, WAIT),
        Err(system_api::loader::Fail::BadImage)
    ));
    assert_eq!(
        state.instance(second.task).state(WAIT).unwrap(),
        system_api::control::State::Debarked
    );
    unsafe {
        *(at as *mut u8) = 0x7f;
    }

    let image_copy =
        env::pie::accord(image, root, Permission::FETCH, system_api::loader::IMAGE).unwrap();
    let (back, seed) = establish::lend_out(entry, system_api::loader::BACK).unwrap();
    let ask = system_api::loader::Ask {
        op: system_api::loader::BUILD,
        image: image_copy,
        offset: 0,
        len: bytes.len() as u64,
        stack: 0,
        count: 0,
        args: [0; system_api::loader::MAX_ARGS],
        back: seed,
    };
    let mut request = [0; system_api::loader::Ask::LEN];
    let n = ask.store_at(&mut request, 0).unwrap();
    Hole::from_raw(entry).push(&request[..n], WAIT).unwrap();
    let mut reply = system_api::loader::Said::EMPTY;
    let said = ipc::hand::Receiver::<system_api::loader::Said>::from_raw(back)
        .recv(&mut reply, WAIT)
        .unwrap();
    assert_eq!(said.status, wire::OK);
    assert!(
        matches!(
            env::pie::revoke(root, image_copy),
            Err(error) if error.source == env::PieFail::Denied
        ),
        "loader: service did not release the remote image loan"
    );
    let _ = env::pie::seal(back);
    let _ = env::pie::release(back);
    // Leave the result unclaimed while keeping its requester alive.
    until(|| state.instance(said.task).state(WAIT) == Ok(system_api::control::State::Dead));
    env::pie::shut(image).unwrap();
    env::pie::release(image).unwrap();
    // The second, claimed Held task is reclaimed when this requester exits.
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
    let mut bytes = alloc::vec![0; 4100];
    bytes[..7].copy_from_slice(&[0x7f, b'E', b'L', b'F', 2, 1, 1]);
    for (at, value) in [(16, 2u16), (18, 243), (52, 64), (54, 56), (56, 1)] {
        bytes[at..at + 2].copy_from_slice(&value.to_le_bytes());
    }
    bytes[20..24].copy_from_slice(&1u32.to_le_bytes());
    bytes[24..32].copy_from_slice(&0x4000u64.to_le_bytes());
    bytes[32..40].copy_from_slice(&64u64.to_le_bytes());
    bytes[64..68].copy_from_slice(&1u32.to_le_bytes());
    bytes[68..72].copy_from_slice(&5u32.to_le_bytes());
    for (offset, value) in [(8, 4096u64), (16, 0x4000), (32, 4), (40, 4), (48, 4096)] {
        bytes[64 + offset..72 + offset].copy_from_slice(&value.to_le_bytes());
    }
    bytes[4096..].copy_from_slice(&[0x6f, 0, 0, 0]);
    bytes
}
