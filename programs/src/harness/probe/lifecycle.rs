use alloc::sync::Arc;
use core::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use env::Wait;
use runtime::env::{room, unit};

fn until(mut done: impl FnMut() -> bool) {
    let deadline = runtime::env::chrono::clock() + 2_000_000_000;
    while !done() {
        assert!(
            runtime::env::chrono::clock() < deadline,
            "lifecycle: timed out"
        );
        room::sleep(core::time::Duration::from_millis(1)).unwrap();
    }
}
fn debark(task: env::TaskId) {
    until(|| match unit::debark(task) {
        Ok(()) => true,
        Err(e) if matches!(e.source, env::UnitFail::Busy) => false,
        Err(e) => panic!("lifecycle: debark {e:?}"),
    });
}

pub fn acceptance() {
    let count = Arc::new(AtomicUsize::new(0));
    let sibling = Arc::new(AtomicUsize::new(0));
    let finish = Arc::new(AtomicBool::new(false));
    let worker = unit::spawn(
        env::TeamId::new(0),
        tick as *const () as usize,
        &[Arc::as_ptr(&count) as usize],
        0,
    )
    .unwrap();
    unit::embark(worker).unwrap();
    let sib = sibling.clone();
    let ending = finish.clone();
    let peer = runtime::core::task::join::closure(move || {
        while !ending.load(Ordering::Acquire) {
            sib.fetch_add(1, Ordering::Release);
            room::sleep(core::time::Duration::from_millis(1)).unwrap();
        }
    });
    until(|| count.load(Ordering::Acquire) > 1);
    for _ in 0..3 {
        debark(worker);
        let stopped = count.load(Ordering::Acquire);
        room::sleep(core::time::Duration::from_millis(12)).unwrap();
        assert_eq!(
            count.load(Ordering::Acquire),
            stopped,
            "lifecycle: timer bypassed Debark"
        );
        assert!(!unit::join(worker, Wait::POLL).unwrap());
        unit::embark(worker).unwrap();
        until(|| count.load(Ordering::Acquire) > stopped);
    }
    debark(worker);
    let before = sibling.load(Ordering::Acquire);
    unit::slay(worker).unwrap();
    until(|| unit::join(worker, Wait::POLL).unwrap_or(true));
    until(|| sibling.load(Ordering::Acquire) > before);
    let running = unit::spawn(
        env::TeamId::new(0),
        spin as *const () as usize,
        &[Arc::as_ptr(&count) as usize],
        0,
    )
    .unwrap();
    debark(running);
    let held = count.load(Ordering::Acquire);
    unit::embark(running).unwrap();
    until(|| count.load(Ordering::Acquire) > held);
    debark(running);
    let stopped = count.load(Ordering::Acquire);
    room::sleep(core::time::Duration::from_millis(12)).unwrap();
    assert_eq!(
        count.load(Ordering::Acquire),
        stopped,
        "lifecycle: running task bypassed Debark"
    );
    unit::embark(running).unwrap();
    until(|| count.load(Ordering::Acquire) > stopped);
    unit::slay(running).unwrap();
    until(|| unit::join(running, Wait::POLL).unwrap_or(true));
    let held = unit::spawn(
        env::TeamId::new(0),
        spin as *const () as usize,
        &[Arc::as_ptr(&count) as usize],
        0,
    )
    .unwrap();
    unit::slay(held).unwrap();
    until(|| unit::join(held, Wait::POLL).unwrap_or(true));
    finish.store(true, Ordering::Release);
    peer.join();
    protocol::debug::put(
        "lifecycle: Debark preserves context, Embark resumes, Slay preserves same-team peer",
    );
}

extern "C" fn tick(args: usize) -> ! {
    // SAFETY: Spawn copies the pointer argument; the parent retains this Atomic
    // until Join confirms the task has exited. Both tasks share one address space.
    let count = unsafe { &*(*(args as *const usize) as *const AtomicUsize) };
    loop {
        count.fetch_add(1, Ordering::Release);
        room::sleep(core::time::Duration::from_millis(3)).unwrap();
    }
}

extern "C" fn spin(args: usize) -> ! {
    // SAFETY: The parent retains the shared Atomic until Join confirms exit.
    let count = unsafe { &*(*(args as *const usize) as *const AtomicUsize) };
    loop {
        count.fetch_add(1, Ordering::Release);
    }
}
