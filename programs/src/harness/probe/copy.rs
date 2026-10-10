use crate::system::loader::{Image, Loader};
use ::resource::raw::{Hole, pies};
use alloc::{sync::Arc, vec::Vec};
use core::sync::atomic::{AtomicBool, Ordering};
use env::pie;
use env::unit;
use env::{Mark, PieToken, ProgramKind, TeamId, Wait};

pub fn acceptance() {
    calls();
    queued();
    waiting();
    concurrent();
    let done = Arc::new(AtomicBool::new(false));
    let finished = done.clone();
    let build = crate::boot::Accounts::take()
        .unwrap()
        .token(env::Name::Call(env::Call::Build))
        .unwrap();
    let doom = crate::boot::Accounts::take()
        .unwrap()
        .token(env::Name::Call(env::Call::Doom))
        .unwrap();
    let doom_mark = Mark::of("copy-doom");
    let mark = Mark::of("copy-build");
    let worker = execution::unit::task::spawn(move || {
        assert!(
            ipc::session::establish::claim(env::TaskId::new(0), mark, Wait::AtMost(2000),).is_ok()
        );
        assert!(
            ipc::session::establish::claim(env::TaskId::new(0), doom_mark, Wait::AtMost(2000),)
                .is_ok()
        );
        elf();
        cache_limits();
        finished.store(true, Ordering::Release);
    });
    env::pie::accord(build, worker.id(), env::Permission::FETCH, mark).unwrap();
    env::pie::accord(doom, worker.id(), env::Permission::FETCH, doom_mark).unwrap();
    worker.join();
    assert!(
        done.load(Ordering::Acquire),
        "copy: loader checks did not complete"
    );
    programs::debug::put(
        "copy: queued ownership, ELF cache identity, zero padding and private rollback passed",
    );
}

// Exercise Await's deadline through real suspension and unrelated group changes.
fn waiting() {
    use ::resource::pile::Pile;
    use env::{AwaitReply, Bit, MailCondition, Source};
    let owner = unit::self_id();
    let mark = Mark::of("copy-wait-group");
    let group = Pile::unseal(true).unwrap();
    let page = pie::unseal(env::UnsealArgs::Pole {
        size: env::PAGE_SIZE,
        shared: true,
    })
    .unwrap();
    let worker = execution::unit::task::spawn(move || {
        let token = ipc::session::establish::claim(owner, mark, Wait::AtMost(2000)).unwrap();
        let group = Pile::from_raw(token);
        let hole = pie::unseal(env::UnsealArgs::hole(Mark::NONE)).unwrap();
        let source = Source::Mail {
            pie: hole,
            condition: MailCondition::Pull,
        };
        env::room::park(20).unwrap();
        group.attach(source).unwrap();
        env::room::park(20).unwrap();
        group.detach(source).unwrap();
        pie::release(hole, env::ReleaseMode::Revoke).unwrap();
    });
    pie::accord(
        group.token(),
        worker.id(),
        env::Permission::FETCH | env::Permission::STORE,
        mark,
    )
    .unwrap();
    let first = Source::Mail {
        pie: page,
        condition: MailCondition::Signal(Bit::FIRST),
    };
    let second = Source::Mail {
        pie: page,
        condition: MailCondition::Signal(Bit::of(1).unwrap()),
    };
    group.attach(first).unwrap();
    group.attach(second).unwrap();
    let start = env::chrono::clock();
    assert_eq!(group.await_(Wait::AtMost(80)).unwrap(), AwaitReply::Pending);
    assert!(
        env::chrono::clock() - start >= 80_000_000,
        "copy: ordinary wake shortened Await deadline"
    );
    worker.join();
    env::mail::ring(page, Bit::FIRST).unwrap();
    env::mail::ring(page, Bit::FIRST).unwrap();
    env::mail::ring(page, Bit::of(1).unwrap()).unwrap();
    assert!(matches!(
        group.await_(Wait::POLL).unwrap(),
        AwaitReply::Source { fail: None, .. }
    ));
    env::mail::hush(page, Bit::FIRST).unwrap();
    env::mail::hush(page, Bit::FIRST).unwrap();
    assert_eq!(
        group.await_(Wait::POLL).unwrap(),
        AwaitReply::Source {
            source: second,
            fail: None
        }
    );
    pie::release(page, env::ReleaseMode::Revoke).unwrap();
    assert_eq!(
        group.await_(Wait::POLL).unwrap(),
        AwaitReply::Source {
            source: first,
            fail: Some(env::MailFail::Denied)
        }
    );
    group.detach(first).unwrap();
    group.detach(second).unwrap();
    programs::debug::put("copy: page bits, source expiry and Await deadline passed");
}

fn calls() {
    use env::{
        HoleLimits, MailCondition, MailFail, Oversize, Permission, PieInfo, PieKind, PullOutcome,
        ReleaseMode, UnsealArgs, VirtAddr,
    };
    let owner = unit::self_id();
    let mark = Mark::new(u64::MAX);
    let token = pie::unseal(UnsealArgs::Hole {
        mark,
        limits: HoleLimits {
            max_len: 8,
            max_messages: 3,
            max_bytes: 12,
        },
    })
    .unwrap();
    let info = ::resource::raw::inspect(token).unwrap();
    assert_eq!(
        (info.token, info.kind, info.owner, info.mark, info.alive),
        (token, PieKind::Hole, owner, mark, true)
    );
    let hole = Hole::from_raw(token);
    hole.push(&[1; 8], Wait::POLL).unwrap();
    hole.push(&[2; 4], Wait::POLL).unwrap();
    assert!(!hole.wait(MailCondition::Push, Wait::POLL).unwrap());
    assert!(!hole.wait(MailCondition::Empty, Wait::POLL).unwrap());
    assert!(
        hole.push(&[0; 9], Wait::POLL)
            .is_err_and(|e| e.source == MailFail::Denied)
    );
    assert!(
        hole.push(&[0], Wait::POLL)
            .is_err_and(|e| e.source == MailFail::Busy)
    );
    let mut bytes = [0; 4];
    assert!(
        hole.pull(&mut bytes, Wait::POLL)
            .is_err_and(|e| e.source == MailFail::Denied)
    );
    assert_eq!(hole.peek().unwrap(), (8, owner, 2));
    assert!(
        env::mail::pull(token, VirtAddr::new(0), 4, Oversize::Discard)
            .is_err_and(|e| e.source == MailFail::Denied)
    );
    assert_eq!(hole.peek().unwrap(), (8, owner, 2));
    assert_eq!(
        hole.pull_with(&mut bytes, Wait::POLL, Oversize::Discard)
            .unwrap(),
        PullOutcome::Discarded {
            len: 8,
            sender: owner
        }
    );
    assert_eq!(hole.peek().unwrap(), (4, owner, 1));
    assert!(hole.wait(MailCondition::Push, Wait::POLL).unwrap());
    assert!(!hole.wait(MailCondition::Empty, Wait::POLL).unwrap());
    let read_only = pie::accord(token, owner, Permission::FETCH, Mark::NONE).unwrap();
    let write_only = pie::accord(token, owner, Permission::STORE, Mark::NONE).unwrap();
    assert!(
        env::mail::wait(read_only, MailCondition::Push, Wait::POLL)
            .is_err_and(|e| e.source == MailFail::Denied)
    );
    assert!(
        env::mail::wait(read_only, MailCondition::Empty, Wait::POLL)
            .is_err_and(|e| e.source == MailFail::Denied)
    );
    assert!(
        env::mail::pull(
            write_only,
            VirtAddr::new(bytes.as_mut_ptr() as usize),
            4,
            Oversize::Discard
        )
        .is_err_and(|e| e.source == MailFail::Denied)
    );
    assert_eq!(hole.peek().unwrap(), (4, owner, 1));
    let group = ::resource::pile::Pile::unseal(false).unwrap();
    assert!(
        group
            .attach(env::Source::Mail {
                pie: read_only,
                condition: MailCondition::Push
            })
            .is_err()
    );
    group
        .attach(env::Source::Mail {
            pie: write_only,
            condition: MailCondition::Push,
        })
        .unwrap();
    assert!(matches!(
        group.await_(Wait::POLL).unwrap().mail(),
        Some((_, MailCondition::Push))
    ));
    group
        .detach(env::Source::Mail {
            pie: write_only,
            condition: MailCondition::Push,
        })
        .unwrap();
    group
        .attach(env::Source::Mail {
            pie: read_only,
            condition: MailCondition::Pull,
        })
        .unwrap();
    assert!(matches!(
        group.await_(Wait::POLL).unwrap().mail(),
        Some((_, MailCondition::Pull))
    ));
    group
        .detach(env::Source::Mail {
            pie: read_only,
            condition: MailCondition::Pull,
        })
        .unwrap();
    group
        .attach(env::Source::Mail {
            pie: write_only,
            condition: MailCondition::Empty,
        })
        .unwrap();
    assert!(group.await_(Wait::POLL).unwrap().is_none());
    assert!(
        group
            .attach(env::Source::Mail {
                pie: group.token(),
                condition: MailCondition::Pull
            })
            .is_err()
    );
    let bell = pie::unseal(UnsealArgs::Nole).unwrap();
    assert!(
        group
            .attach(env::Source::Mail {
                pie: bell,
                condition: MailCondition::Push
            })
            .is_err()
    );
    assert!(
        group
            .attach(env::Source::Mail {
                pie: bell,
                condition: MailCondition::Empty
            })
            .is_err()
    );
    assert_eq!(
        hole.pull_with(&mut bytes, Wait::POLL, Oversize::Discard)
            .unwrap(),
        PullOutcome::Received {
            len: 4,
            sender: owner
        }
    );
    assert_eq!(bytes, [2; 4]);
    assert!(matches!(
        group.await_(Wait::POLL).unwrap().mail(),
        Some((_, MailCondition::Empty))
    ));
    pie::seal(token).unwrap();
    let closed = ::resource::raw::inspect(token).unwrap();
    assert!(!closed.alive);
    assert_eq!((closed.owner, closed.mark), (owner, mark));
    assert!(!::resource::raw::alive(token));
    assert!(::resource::raw::reserve(token).is_err());
    for token in [token, group.token(), bell] {
        pie::release(token, ReleaseMode::Revoke).unwrap();
    }

    let mut noles = Vec::new();
    for _ in 0..20 {
        noles.push(pie::unseal(UnsealArgs::Nole).unwrap());
    }
    let mut words = [0; PieInfo::WORDS * 4];
    let buf = VirtAddr::new(words.as_mut_ptr() as usize);
    let count = pie::collect(noles[0], buf, 4).unwrap();
    assert_eq!(count, 4);
    let record = |words: &[usize], at: usize| {
        PieInfo::from_words(
            words[at * PieInfo::WORDS..(at + 1) * PieInfo::WORDS]
                .try_into()
                .unwrap(),
        )
        .unwrap()
    };
    assert_eq!(record(&words, 0).token, noles[1]);
    let after = record(&words, 3).token;
    assert_eq!(after, noles[4]);
    pie::release(noles[0], ReleaseMode::Revoke).unwrap();
    pie::release(noles[1], ReleaseMode::Revoke).unwrap();
    let count = pie::collect(after, buf, 4).unwrap();
    assert_eq!(count, 4);
    for i in 0..count {
        let info = record(&words, i);
        assert_eq!(
            (info.token, info.kind, info.owner, info.alive),
            (noles[i + 5], PieKind::Nole, owner, true)
        );
    }
    for token in noles.into_iter().skip(2) {
        pie::release(token, ReleaseMode::Revoke).unwrap();
    }
    programs::debug::put(
        "copy: unified creation, bounded queues, Pull policy, wait permissions and token cursor passed",
    );
}

fn queued() {
    let owner = unit::self_id();
    let entry = pie::unseal(env::UnsealArgs::hole(Mark::of("copy-queued"))).unwrap();
    let raw_owner = owner.get();
    let writer = execution::unit::task::spawn(move || {
        let entry = ipc::session::establish::claim(
            env::TaskId::new(raw_owner),
            Mark::of("copy-queued"),
            Wait::AtMost(1000),
        )
        .unwrap();
        let mut data = alloc::vec![0; 128];
        for sequence in 0..4u8 {
            data.fill(sequence);
            Hole::from_raw(entry).push(&data, Wait::POLL).unwrap();
        }
        assert!(
            Hole::from_raw(entry)
                .push(&data, Wait::POLL)
                .is_err_and(|error| error.source.is_busy()),
            "copy: queue lost its bound"
        );
        data.fill(255);
    });
    let sender = writer.id();
    ::resource::port::ship(entry, sender, env::Access::STORE, env::Policy::NONE).unwrap();
    writer.join();
    let mut short = [0; 4];
    assert!(Hole::from_raw(entry).pull(&mut short, Wait::POLL).is_err());
    let mut bytes = [0; 128];
    for sequence in 0..4u8 {
        assert_eq!(
            Hole::from_raw(entry).pull(&mut bytes, Wait::POLL).unwrap(),
            (bytes.len(), sender)
        );
        assert!(
            bytes.iter().all(|&byte| byte == sequence),
            "copy: queued bytes changed after sender exit"
        );
    }
    let hole = Hole::from_raw(entry);
    for cycle in 0..16u8 {
        assert!(hole.wait(env::MailCondition::Empty, Wait::POLL).unwrap());
        assert!(!hole.wait(env::MailCondition::Pull, Wait::POLL).unwrap());
        for _ in 0..4 {
            env::mail::ring(entry, env::Bit::FIRST).unwrap();
        }
        assert!(env::mail::ring(entry, env::Bit::FIRST).is_err());
        assert!(hole.wait(env::MailCondition::Pull, Wait::POLL).unwrap());
        assert!(!hole.wait(env::MailCondition::Empty, Wait::POLL).unwrap());
        assert!(hole.push(&[cycle], Wait::POLL).is_err());
        for _ in 0..4 {
            env::mail::hush(entry, env::Bit::FIRST).unwrap();
        }
        assert!(env::mail::hush(entry, env::Bit::FIRST).is_err());
        assert!(hole.wait(env::MailCondition::Empty, Wait::POLL).unwrap());
        hole.push(&[cycle], Wait::POLL).unwrap();
        assert_eq!(hole.pull(&mut bytes, Wait::POLL).unwrap(), (1, owner));
        assert_eq!(bytes[0], cycle);
    }
    pie::seal(entry).unwrap();
    pie::release(entry, env::ReleaseMode::Revoke).unwrap();
}

fn concurrent() {
    let owner = unit::self_id();
    let entry = pie::unseal(env::UnsealArgs::hole(Mark::of("copy-concurrent"))).unwrap();
    let mut workers = Vec::new();
    for producer in 0..4u8 {
        let worker = execution::unit::task::spawn(move || {
            let token = ipc::session::establish::claim(
                owner,
                Mark::of("copy-concurrent"),
                Wait::AtMost(2000),
            )
            .unwrap();
            for sequence in 0..32u8 {
                Hole::from_raw(token)
                    .push(&[producer, sequence], Wait::AtMost(2000))
                    .unwrap();
            }
        });
        ::resource::port::ship(entry, worker.id(), env::Access::STORE, env::Policy::NONE).unwrap();
        workers.push(worker);
    }
    let mut counts = [0u8; 4];
    let mut bytes = [0; 2];
    for _ in 0..128 {
        let (size, sender) = Hole::from_raw(entry)
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
    pie::release(entry, env::ReleaseMode::Revoke).unwrap();
    programs::debug::put(
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
    let mut loader = Loader::new();
    let mut bytes = image();
    let before = tokens();
    drop(
        loader
            .build(Image {
                bytes: &bytes,
                kind: ProgramKind::User,
            })
            .unwrap(),
    );
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
    let page = view;
    let (at, size) = ::resource::raw::open(page).unwrap();
    assert_eq!(size, 8192);
    // SAFETY: Open holds a readable mapping of size bytes until Shut.
    let data = unsafe { core::slice::from_raw_parts(at as *const u8, size) };
    assert_eq!(&data[2..6], &[0x13, 0, 0, 0]);
    assert!(data[..2].iter().chain(&data[6..]).all(|&byte| byte == 0));
    assert!(
        execution::memory::map(TeamId::new(0), 0, size, source, 0, 6).is_err(),
        "copy: shared source can be widened"
    );
    env::pie::shut(page).unwrap();
    pie::release(view, env::ReleaseMode::Revoke).unwrap();
    drop(
        loader
            .build(Image {
                bytes: &bytes.clone(),
                kind: ProgramKind::User,
            })
            .unwrap(),
    );
    assert_eq!(
        tokens(),
        cached,
        "copy: equal content at another address missed cache"
    );
    let mut shifted = bytes.clone();
    segment(&mut shifted, 64, [5, 4096, 0x10000, 6, 4103]);
    drop(
        loader
            .build(Image {
                bytes: &shifted,
                kind: ProgramKind::User,
            })
            .unwrap(),
    );
    assert_eq!(
        tokens(),
        cached,
        "copy: equivalent zero padding missed cache"
    );
    let mut relocated = bytes.clone();
    relocated[24..32].copy_from_slice(&0x30002u64.to_le_bytes());
    segment(&mut relocated, 64, [5, 4098, 0x30002, 4, 4101]);
    drop(
        loader
            .build(Image {
                bytes: &relocated,
                kind: ProgramKind::User,
            })
            .unwrap(),
    );
    assert_eq!(tokens(), cached, "copy: destination VA split shared cache");
    bytes[4098] = 0x93;
    drop(
        loader
            .build(Image {
                bytes: &bytes,
                kind: ProgramKind::User,
            })
            .unwrap(),
    );
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
    let writable = loader
        .build(Image {
            bytes: &bytes,
            kind: ProgramKind::User,
        })
        .unwrap();
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
    drop(loader);
    assert_eq!(tokens(), before, "copy: loader cache survived its owner");

    let mut loader = Loader::new();
    let heirs = {
        let mut page = [0u64; 64];
        env::unit::scan(
            env::TeamId::new(0),
            env::VirtAddr::new(page.as_mut_ptr() as usize),
            page.len(),
        )
        .expect("Scan heirs")
    };
    assert!(
        loader
            .build(Image {
                bytes: b"invalid",
                kind: ProgramKind::User
            })
            .is_err_and(|e| e.source == env::UnitFail::BadImage)
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
        heirs
    );
    let minted = loader
        .build(Image {
            bytes: &bytes,
            kind: ProgramKind::User,
        })
        .unwrap();
    let team = minted.team();
    drop(loader);
    let task = minted.spawn(&[], 0).unwrap();
    assert_eq!(
        tokens(),
        before,
        "copy: committed sources survived loader drop"
    );
    env::room::doom(task).unwrap();
    assert!(unit::join_task(task, Wait::AtMost(2000)).unwrap());
    unit::oust(team).unwrap();
}

fn cache_limits() {
    let before = tokens();
    let mut loader = Loader::new();
    let small = image();
    let held = loader
        .build(Image {
            bytes: &small,
            kind: ProgramKind::User,
        })
        .unwrap();
    let mut large = small.clone();
    segment(&mut large, 64, [5, 4098, 0x10002, 4, 256 * 4096 - 2]);
    drop(
        loader
            .build(Image {
                bytes: &large,
                kind: ProgramKind::User,
            })
            .unwrap(),
    );
    assert_eq!(
        tokens().len(),
        before.len() + 1,
        "copy: cache exceeded page budget"
    );
    segment(&mut large, 64, [5, 4098, 0x10002, 4, 257 * 4096 - 2]);
    drop(
        loader
            .build(Image {
                bytes: &large,
                kind: ProgramKind::User,
            })
            .unwrap(),
    );
    assert_eq!(
        tokens().len(),
        before.len() + 1,
        "copy: oversized source entered cache"
    );
    let team = held.team();
    let task = held.spawn(&[], 0).unwrap();
    env::room::doom(task).unwrap();
    assert!(unit::join_task(task, Wait::AtMost(2000)).unwrap());
    unit::oust(team).unwrap();
    drop(loader);
    assert_eq!(tokens(), before, "copy: cache eviction leaked roots");
}
