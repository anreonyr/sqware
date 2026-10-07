use ::schedule::{Cursor, Progress, Schedule};
use super::fixture::Fixture;
use alloc::sync::Arc;
use core::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use env::{Mark, Permission, TaskId, Wait};
use protocol::system::{control, loader as call, operator::client::Face as Operator};
use ipc::session::establish;
use ::resource::raw::Hole;

const WAIT: Wait = Wait::AtMost(2000);

pub fn acceptance(assembly: &mut Fixture, operator: &Operator) {
    let road = call::DIR.try_join(call::Grant::Build.name()).unwrap();
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
        let face = control::Face::of(establish::find(root, control::Grant::State.mark()).unwrap())
            .unwrap();
        assert_eq!(
            face.instance(TaskId::new(t.load(Ordering::Acquire)))
                .state(WAIT),
            Err(control::Fail::Denied)
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
    let entry = assembly
        .resources
        .read::<crate::system::run::loading::answer::Inbox>()
        .unwrap()
        .entry
        .unwrap();
    env::pie::accord(entry, worker.id(), Permission::STORE, Mark::NONE).unwrap();
    for grant in [
        control::Grant::State,
        control::Grant::Embark,
        control::Grant::Debark,
        control::Grant::Ruin,
    ] {
        let entry = assembly
            .resources
            .read::<crate::system::control::serve::watch::Watch>()
            .unwrap()
            .faces[grant.index()]
        .unwrap();
        env::pie::accord(entry, worker.id(), Permission::STORE, Mark::NONE).unwrap();
        if grant == control::Grant::State {
            env::pie::accord(entry, peer.id(), Permission::STORE, Mark::NONE).unwrap();
        }
    }
    {
        let roster = assembly
            .resources
            .read::<crate::system::identity::client::install::Roster>()
            .unwrap();
        roster.inherit(worker.id(), root).unwrap();
        roster.inherit(peer.id(), root).unwrap();
    }
    let mut schedule = Schedule::new();
    schedule
        .add_plan(
            "loader",
            0u8,
            crate::system::run::loading::schedule::frame().unwrap(),
        )
        .unwrap();
    schedule
        .add_system("receive", 1, crate::system::control::serve::answer::receive)
        .unwrap();
    schedule
        .add_system(
            "instances",
            2,
            crate::system::control::serve::instance::answer,
        )
        .unwrap();
    schedule
        .add_system("reap", 3, crate::system::run::instances::reap)
        .unwrap();
    schedule
        .add_plan(
            "instance.hooks",
            4,
            crate::system::run::hooks::instance().unwrap(),
        )
        .unwrap();
    schedule
        .add_system("launch.completed", 5, crate::system::run::launch::completed)
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
                .read::<crate::system::run::resource::Resources>()
                .unwrap()
                .runtime_road(task)
                .is_some()
        {
            registered = true;
        }
        let clean = assembly
            .resources
            .read::<crate::system::control::serve::unit::Control>()
            .unwrap()
            .instances
            .iter()
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
    protocol::debug::put(
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
    use protocol::wire::message::Message;
    let entry = establish::find(root, call::Grant::Build.mark()).unwrap();
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
    let face = |grant: control::Grant| {
        control::Face::of(establish::find(root, grant.mark()).unwrap()).unwrap()
    };
    let state = face(control::Grant::State);
    assert_eq!(
        state.instance(first.task).state(WAIT).unwrap(),
        control::State::Debarked
    );
    assert!(
        env::unit::join(first.task, Wait::POLL).is_err_and(|e| e.source == env::UnitFail::Denied)
    );
    face(control::Grant::Embark)
        .instance(first.task)
        .embark(WAIT)
        .unwrap();
    assert_eq!(
        state.instance(first.task).state(WAIT).unwrap(),
        control::State::Ready
    );
    until(|| {
        face(control::Grant::Debark)
            .instance(first.task)
            .debark(WAIT)
            .is_ok()
    });
    assert_eq!(
        state.instance(first.task).state(WAIT).unwrap(),
        control::State::Debarked
    );
    face(control::Grant::Embark)
        .instance(first.task)
        .embark(WAIT)
        .unwrap();
    face(control::Grant::Ruin)
        .instance(first.task)
        .ruin(WAIT)
        .unwrap();
    until(|| state.instance(first.task).state(WAIT) == Ok(control::State::Dead));
    assert_eq!(
        state.instance(second.task).state(WAIT).unwrap(),
        control::State::Debarked
    );
    assert!(matches!(
        loader.build(image, size, 1, &[], 0, WAIT),
        Err(call::Fail::BadImage)
    ));
    // SAFETY: the caller retains write access while the service uses an independent snapshot.
    unsafe {
        *(at as *mut u8) = 0;
    }
    assert!(matches!(
        loader.build(image, 0, bytes.len(), &[], 0, WAIT),
        Err(call::Fail::BadImage)
    ));
    assert_eq!(
        state.instance(second.task).state(WAIT).unwrap(),
        control::State::Debarked
    );
    unsafe {
        *(at as *mut u8) = 0x7f;
    }

    let image_copy = env::pie::accord(image, root, Permission::FETCH, call::frame::IMAGE).unwrap();
    let (back, seed) = establish::lend_out(entry, call::frame::BACK).unwrap();
    let ask = call::frame::Ask {
        op: call::frame::BUILD,
        image: image_copy,
        offset: 0,
        len: bytes.len() as u64,
        stack: 0,
        count: 0,
        args: [0; call::frame::MAX_ARGS],
        back: seed,
    };
    let mut request = [0; call::frame::Ask::LEN];
    let n = ask.store_at(&mut request, 0).unwrap();
    Hole::from_raw(entry)
        .push(&request[..n], WAIT)
        .unwrap();
    let mut reply = call::frame::Said::EMPTY;
    let said = ipc::hand::Receiver::<call::frame::Said>::from_raw(back)
        .recv(&mut reply, WAIT)
        .unwrap();
    assert_eq!(said.status, protocol::wire::OK);
    let _ = env::pie::seal(back);
    let _ = env::pie::release(back);
    // Leave the result unclaimed while keeping its requester alive.
    until(|| state.instance(said.task).state(WAIT) == Ok(control::State::Dead));
    env::pie::shut(image).unwrap();
    env::pie::release(image).unwrap();
    // The second, claimed Held task is reclaimed when this requester exits.
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
