use alloc::{sync::Arc, vec::Vec};
use core::sync::atomic::{AtomicBool, Ordering};
use env::{Mark, PieToken, ProgramKind, TeamId, Wait};
use env::unit;
use env::pie;
use runtime::core::res::pie::{HolePie, PolePie, pies};
use runtime::core::adapt;

pub fn acceptance() {
    queued();
    concurrent();
    let done = Arc::new(AtomicBool::new(false));
    let finished = done.clone();
    let build = crate::boot::Accounts::take().unwrap()
        .token(env::Name::Call(env::Call::Build)).unwrap();
    let mark = Mark::of("copy-build");
    let worker = runtime::core::task::join::closure(move || {
        assert!(protocol::communication::session::establish::claim(
            env::TaskId::new(0), mark, Wait::AtMost(2000),
        ).is_some());
        elf();
        finished.store(true, Ordering::Release);
    });
    env::pie::accord(build, worker.id(), env::Permission::FETCH, mark).unwrap();
    worker.join();
    assert!(
        done.load(Ordering::Acquire),
        "copy: loader checks did not complete"
    );
    protocol::debug::put(
        "copy: queued ownership, ELF cache identity, zero padding and private rollback passed",
    );
}

fn queued() {
    let owner = unit::self_id();
    let entry = pie::unseal_hole(Mark::of("copy-queued")).unwrap();
    let raw_owner = owner.get();
    let writer = runtime::core::task::join::closure(move || {
        let entry = protocol::communication::session::establish::claim(
            env::TaskId::new(raw_owner),
            Mark::of("copy-queued"),
            Wait::AtMost(1000),
        )
        .unwrap();
        let mut data = alloc::vec![0; 128];
        for sequence in 0..4u8 {
            data.fill(sequence);
            HolePie::from_token(entry).push(&data, Wait::POLL).unwrap();
        }
        assert!(
            HolePie::from_token(entry)
                .push(&data, Wait::POLL)
                .is_err_and(|error| error.source.is_busy()),
            "copy: queue lost its bound"
        );
        data.fill(255);
    });
    let sender = writer.id();
    runtime::core::res::port::ship(
        &HolePie::from_token(entry),
        sender,
        env::Access::STORE,
        env::Policy::NONE,
    )
    .unwrap();
    writer.join();
    let mut short = [0; 4];
    assert!(
        HolePie::from_token(entry)
            .pull(&mut short, Wait::POLL)
            .is_err()
    );
    let mut bytes = [0; 128];
    for sequence in 0..4u8 {
        assert_eq!(
            HolePie::from_token(entry)
                .pull(&mut bytes, Wait::POLL)
                .unwrap(),
            (bytes.len(), sender)
        );
        assert!(
            bytes.iter().all(|&byte| byte == sequence),
            "copy: queued bytes changed after sender exit"
        );
    }
    let hole = HolePie::from_token(entry);
    for cycle in 0..16u8 {
        assert!(hole.wait(env::HoleDir::Push, Wait::POLL).unwrap());
        assert!(!hole.wait(env::HoleDir::Pull, Wait::POLL).unwrap());
        for _ in 0..4 {
            env::mail::ring(entry).unwrap();
        }
        assert!(env::mail::ring(entry).is_err());
        assert!(hole.wait(env::HoleDir::Pull, Wait::POLL).unwrap());
        assert!(!hole.wait(env::HoleDir::Push, Wait::POLL).unwrap());
        assert!(hole.push(&[cycle], Wait::POLL).is_err());
        for _ in 0..4 {
            env::mail::hush(entry).unwrap();
        }
        assert!(env::mail::hush(entry).is_err());
        assert!(hole.wait(env::HoleDir::Push, Wait::POLL).unwrap());
        hole.push(&[cycle], Wait::POLL).unwrap();
        assert_eq!(hole.pull(&mut bytes, Wait::POLL).unwrap(), (1, owner));
        assert_eq!(bytes[0], cycle);
    }
    pie::seal(entry).unwrap();
    pie::release(entry).unwrap();
}

fn concurrent() {
    let owner = unit::self_id();
    let entry = pie::unseal_hole(Mark::of("copy-concurrent")).unwrap();
    let mut workers = Vec::new();
    for producer in 0..4u8 {
        let worker = runtime::core::task::join::closure(move || {
            let token = protocol::communication::session::establish::claim(
                owner,
                Mark::of("copy-concurrent"),
                Wait::AtMost(2000),
            )
            .unwrap();
            for sequence in 0..32u8 {
                HolePie::from_token(token)
                    .push(&[producer, sequence], Wait::AtMost(2000))
                    .unwrap();
            }
        });
        runtime::core::res::port::ship(
            &HolePie::from_token(entry),
            worker.id(),
            env::Access::STORE,
            env::Policy::NONE,
        )
        .unwrap();
        workers.push(worker);
    }
    let mut counts = [0u8; 4];
    let mut bytes = [0; 2];
    for _ in 0..128 {
        let (size, sender) = HolePie::from_token(entry)
            .pull(&mut bytes, Wait::AtMost(2000))
            .unwrap();
        assert_eq!(size, 2);
        let producer = bytes[0] as usize;
        assert!(producer < workers.len());
        assert_eq!(sender, workers[producer].id());
        assert_eq!(bytes[1], counts[producer], "copy: producer FIFO changed");
        counts[producer] += 1;
    }
    assert_eq!(counts, [32; 4]);
    for worker in workers {
        worker.join();
    }
    pie::seal(entry).unwrap();
    pie::release(entry).unwrap();
    protocol::debug::put(
        "copy: four concurrent producers delivered 128 frames in producer FIFO order",
    );
}

fn tokens() -> Vec<PieToken> {
    pies().map(|pie| pie.token).collect()
}

fn image() -> Vec<u8> {
    let mut bytes = alloc::vec![0; 4096 + 6];
    bytes[..7].copy_from_slice(&[0x7f, b'E', b'L', b'F', 2, 1, 1]);
    for (at, value) in [(16, 2u16), (18, 243), (52, 64), (54, 56), (56, 1)] {
        bytes[at..at + 2].copy_from_slice(&value.to_le_bytes());
    }
    bytes[20..24].copy_from_slice(&1u32.to_le_bytes());
    bytes[24..32].copy_from_slice(&0x10002u64.to_le_bytes());
    bytes[32..40].copy_from_slice(&64u64.to_le_bytes());
    segment(&mut bytes, 64, [5, 4098, 0x10002, 4, 4101]);
    bytes[4098..4102].copy_from_slice(&[0x13, 0, 0, 0]);
    bytes
}

fn segment(bytes: &mut [u8], at: usize, values: [u64; 5]) {
    bytes[at..at + 4].copy_from_slice(&1u32.to_le_bytes());
    bytes[at + 4..at + 8].copy_from_slice(&(values[0] as u32).to_le_bytes());
    for (offset, value) in [
        (8, values[1]),
        (16, values[2]),
        (32, values[3]),
        (40, values[4]),
        (48, 1),
    ] {
        bytes[at + offset..at + offset + 8].copy_from_slice(&value.to_le_bytes());
    }
}

fn elf() {
    let mut bytes = image();
    let before = tokens();
    drop(runtime::core::loader::build(&bytes, ProgramKind::User).unwrap());
    let cached = tokens();
    let added: Vec<_> = cached
        .iter()
        .filter(|token| !before.contains(token))
        .copied()
        .collect();
    assert_eq!(added.len(), 1, "copy: expected one shared ELF source");
    let source = added[0];
    let view =
        env::pie::accord(source, unit::self_id(), env::Permission::FETCH, Mark::NONE).unwrap();
    let page = PolePie::from_token(view);
    let (at, size) = page.open().unwrap();
    assert_eq!(size, 8192);
    // SAFETY: Open holds a readable mapping of size bytes until Shut.
    let data = unsafe { core::slice::from_raw_parts(at as *const u8, size) };
    assert_eq!(&data[2..6], &[0x13, 0, 0, 0]);
    assert!(data[..2].iter().chain(&data[6..]).all(|&byte| byte == 0));
    assert!(
        adapt::map(TeamId::new(0), 0, size, source, 0, 6).is_err(),
        "copy: shared source can be widened"
    );
    page.shut().unwrap();
    pie::release(view).unwrap();
    drop(runtime::core::loader::build(&bytes.clone(), ProgramKind::User).unwrap());
    assert_eq!(
        tokens(),
        cached,
        "copy: equal content at another address missed cache"
    );
    let mut shifted = bytes.clone();
    segment(&mut shifted, 64, [5, 4096, 0x10000, 6, 4103]);
    drop(runtime::core::loader::build(&shifted, ProgramKind::User).unwrap());
    assert_eq!(
        tokens(),
        cached,
        "copy: equivalent zero padding missed cache"
    );
    bytes[4098] = 0x93;
    drop(runtime::core::loader::build(&bytes, ProgramKind::User).unwrap());
    assert_eq!(
        tokens().len(),
        cached.len() + 1,
        "copy: changed content reused stale cache"
    );
    let stable = tokens();
    bytes.resize(8196, 0);
    bytes[56..58].copy_from_slice(&2u16.to_le_bytes());
    segment(&mut bytes, 120, [6, 8193, 0x20001, 3, 4101]);
    bytes[8193..8196].copy_from_slice(&[7, 8, 9]);
    let writable = runtime::core::loader::build(&bytes, ProgramKind::User).unwrap();
    assert_eq!(
        tokens().len(),
        stable.len() + 1,
        "copy: private segment did not get its own source"
    );
    drop(writable);
    assert_eq!(
        tokens(),
        stable,
        "copy: unpublished private image leaked its source"
    );
}
