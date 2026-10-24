use super::*;
use system_api::bootstrap::{Fail, Frame, ROLE, VERSION};
use wire::Message;

fn reset() {
    EFFECTS.with(|effects| *effects.borrow_mut() = Effects::default());
}

fn frame(provider: TaskId, system: TaskId, role: u8, version: u8) -> Vec<u8> {
    let value = Frame {
        version,
        role,
        provider,
        system,
    };
    let mut out = <Frame as Message>::EMPTY.to_vec();
    let len = value.store(&mut out).unwrap();
    out.truncate(len);
    out
}

fn set_pull(provider: TaskId, system: TaskId, role: u8, version: u8, from: TaskId, len: usize) {
    EFFECTS.with(|effects| {
        let mut effects = effects.borrow_mut();
        effects.pull_bytes = frame(provider, system, role, version);
        effects.pull_from = from;
        effects.pull_len = len;
    });
}

fn set_pie(kind: PieKind, permission: Permission, owner: TaskId, vestor: TaskId) {
    EFFECTS.with(|effects| {
        effects.borrow_mut().pies = vec![PieInfo {
            token: PieToken::mint(21),
            mark: system_api::bootstrap::MARK,
            owner,
            vestor,
            alive: true,
            kind,
            permission,
        }];
    });
}

fn make_jobs() -> system::assembly::Jobs {
    let first = system::assembly::Reference { task: TaskId::new(7) };
    let second = system::assembly::Reference { task: TaskId::new(8) };
    system::assembly::Jobs {
        active: Some(first),
        launches: vec![
            system::assembly::Launch {
                instance: system::assembly::Launched {
                    reference: first,
                    task: TaskId::new(7),
                },
                installation: system::assembly::Install {
                    delivery: system::assembly::Delivery { owner: TaskId::new(3) },
                    bootstrap: None,
                },
            },
            system::assembly::Launch {
                instance: system::assembly::Launched {
                    reference: second,
                    task: TaskId::new(8),
                },
                installation: system::assembly::Install {
                    delivery: system::assembly::Delivery { owner: TaskId::new(3) },
                    bootstrap: None,
                },
            },
        ],
    }
}

fn local() {
    EFFECTS.with(|effects| {
        let mut effects = effects.borrow_mut();
        effects.me = TaskId::new(1);
        effects.creator = TaskId::new(1);
    });
}

fn assert_same_handle() {
    EFFECTS.with(|effects| {
        let effects = effects.borrow();
        assert!(!effects.operations.is_empty());
        assert!(effects.operations.iter().all(|(_, token)| *token == PieToken::mint(1)),
            "failure and retry must keep the original root");
    });
}

#[test]
fn prepare_boundary_failures_retry_same_handle() {
    reset();
    local();
    let mut jobs = make_jobs();
    EFFECTS.with(|effects| effects.borrow_mut().fail_push = true);
    assert_eq!(
        system::assembly::bootstrap::prepare(&mut jobs, TaskId::new(4)),
        Err("bootstrap push")
    );
    EFFECTS.with(|effects| {
        let effects = effects.borrow();
        assert_eq!((effects.unseals, effects.pushes, effects.ships, effects.narrows), (1, 0, 0, 0));
    });
    EFFECTS.with(|effects| effects.borrow_mut().fail_push = false);
    system::assembly::bootstrap::prepare(&mut jobs, TaskId::new(4)).unwrap();
    EFFECTS.with(|effects| {
        let effects = effects.borrow();
        assert_eq!((effects.unseals, effects.pushes, effects.ships, effects.narrows), (1, 1, 1, 1));
    });
    assert_same_handle();

    reset();
    local();
    let mut jobs = make_jobs();
    EFFECTS.with(|effects| effects.borrow_mut().fail_ship = true);
    assert_eq!(
        system::assembly::bootstrap::prepare(&mut jobs, TaskId::new(4)),
        Err("bootstrap ship")
    );
    EFFECTS.with(|effects| {
        let effects = effects.borrow();
        assert_eq!((effects.unseals, effects.pushes, effects.ships, effects.narrows), (1, 1, 0, 0));
    });
    EFFECTS.with(|effects| effects.borrow_mut().fail_ship = false);
    system::assembly::bootstrap::prepare(&mut jobs, TaskId::new(4)).unwrap();
    EFFECTS.with(|effects| {
        let effects = effects.borrow();
        assert_eq!((effects.unseals, effects.pushes, effects.ships, effects.narrows), (1, 1, 1, 1));
    });
    assert_same_handle();

    reset();
    local();
    let mut jobs = make_jobs();
    EFFECTS.with(|effects| effects.borrow_mut().fail_narrow = true);
    assert_eq!(
        system::assembly::bootstrap::prepare(&mut jobs, TaskId::new(4)),
        Err("bootstrap restrict")
    );
    EFFECTS.with(|effects| {
        let effects = effects.borrow();
        assert_eq!((effects.unseals, effects.pushes, effects.ships, effects.narrows), (1, 1, 1, 0));
    });
    EFFECTS.with(|effects| effects.borrow_mut().fail_narrow = false);
    system::assembly::bootstrap::prepare(&mut jobs, TaskId::new(4)).unwrap();
    EFFECTS.with(|effects| {
        let effects = effects.borrow();
        assert_eq!((effects.unseals, effects.pushes, effects.ships, effects.narrows), (1, 1, 1, 1));
    });
    assert_same_handle();

    reset();
    local();
    let mut jobs = make_jobs();
    EFFECTS.with(|effects| effects.borrow_mut().fail_unseal = true);
    assert_eq!(
        system::assembly::bootstrap::prepare(&mut jobs, TaskId::new(4)),
        Err("bootstrap hole")
    );
    EFFECTS.with(|effects| assert_eq!(effects.borrow().unseals, 0));
    EFFECTS.with(|effects| effects.borrow_mut().fail_unseal = false);
    system::assembly::bootstrap::prepare(&mut jobs, TaskId::new(4)).unwrap();
    EFFECTS.with(|effects| assert_eq!(effects.borrow().unseals, 1));
}

#[test]
fn prepare_retry_after_success_does_not_republish() {
    reset();
    local();
    let mut jobs = make_jobs();
    system::assembly::bootstrap::prepare(&mut jobs, TaskId::new(4)).unwrap();
    let before = EFFECTS.with(|effects| {
        let effects = effects.borrow();
        (effects.unseals, effects.pushes, effects.ships, effects.narrows, effects.pushed.len())
    });
    system::assembly::bootstrap::prepare(&mut jobs, TaskId::new(4)).unwrap();
    let after = EFFECTS.with(|effects| {
        let effects = effects.borrow();
        (effects.unseals, effects.pushes, effects.ships, effects.narrows, effects.pushed.len())
    });
    assert_eq!(before, after);
}

#[test]
fn release_failure_keeps_carrier_then_releases_once() {
    reset();
    local();
    let mut jobs = make_jobs();
    system::assembly::bootstrap::prepare(&mut jobs, TaskId::new(4)).unwrap();
    EFFECTS.with(|effects| effects.borrow_mut().fail_release = true);
    assert_eq!(
        system::assembly::bootstrap::release(&mut jobs),
        Err("bootstrap release")
    );
    EFFECTS.with(|effects| assert_eq!(effects.borrow().releases, 0));
    EFFECTS.with(|effects| effects.borrow_mut().fail_release = false);
    assert_eq!(system::assembly::bootstrap::release(&mut jobs), Ok(()));
    assert_eq!(system::assembly::bootstrap::release(&mut jobs), Ok(()));
    EFFECTS.with(|effects| assert_eq!(effects.borrow().releases, 1));
    assert_same_handle();
}

#[test]
fn every_partial_prepare_is_releasable_without_publishing_later_steps() {
    for boundary in 0..4 {
        reset();
        local();
        let mut jobs = make_jobs();
        EFFECTS.with(|effects| {
            let mut effects = effects.borrow_mut();
            effects.fail_unseal = boundary == 0;
            effects.fail_push = boundary == 1;
            effects.fail_ship = boundary == 2;
            effects.fail_narrow = boundary == 3;
        });
        assert!(system::assembly::bootstrap::prepare(&mut jobs, TaskId::new(4)).is_err());
        let before = EFFECTS.with(|effects| {
            let effects = effects.borrow();
            assert_eq!((effects.unseals, effects.pushes, effects.ships, effects.narrows),
                (usize::from(boundary > 0), usize::from(boundary > 1), usize::from(boundary > 2), 0));
            (effects.unseals, effects.pushes, effects.ships, effects.narrows)
        });
        system::assembly::bootstrap::release(&mut jobs).unwrap();
        system::assembly::bootstrap::release(&mut jobs).unwrap();
        EFFECTS.with(|effects| {
            let effects = effects.borrow();
            assert_eq!((effects.unseals, effects.pushes, effects.ships, effects.narrows), before);
            assert_eq!(effects.releases, usize::from(boundary > 0));
        });
        if boundary > 0 { assert_same_handle(); }
    }
}

#[test]
fn schedule_retries_bootstrap_before_later_prepare_steps() {
    use schedule::{Cursor, Progress, ResMut, Resources, Schedule};
    for boundary in 0..4 {
        reset();
        local();
        EFFECTS.with(|effects| {
            let mut effects = effects.borrow_mut();
            effects.fail_unseal = boundary == 0;
            effects.fail_push = boundary == 1;
            effects.fail_ship = boundary == 2;
            effects.fail_narrow = boundary == 3;
        });
        let mut resources = Resources::new();
        resources.insert(make_jobs()).unwrap();
        resources.insert(0usize).unwrap();
        let mut sequence = Schedule::sequence();
        sequence.system("bootstrap", |mut jobs: ResMut<system::assembly::Jobs>| {
            system::assembly::bootstrap::advance(&mut jobs, TaskId::new(4))
        }).unwrap();
        sequence.system("after", |mut calls: ResMut<usize>| {
            *calls += 1;
            Ok(Progress::Done)
        }).unwrap();
        let mut plan = sequence.build().unwrap();
        plan.prepare(&resources);
        let mut cursor = Cursor::default();
        assert_eq!(plan.advance(&mut cursor, &resources).unwrap(), Progress::Pending);
        assert_eq!(*resources.read::<usize>().unwrap(), 0);
        EFFECTS.with(|effects| {
            let mut effects = effects.borrow_mut();
            effects.fail_unseal = false;
            effects.fail_push = false;
            effects.fail_ship = false;
            effects.fail_narrow = false;
        });
        assert_eq!(plan.advance(&mut cursor, &resources).unwrap(), Progress::Done);
        assert_eq!(*resources.read::<usize>().unwrap(), 1);
        assert_same_handle();
        EFFECTS.with(|effects| effects.borrow_mut().now = u64::MAX);
        assert_eq!(system::assembly::bootstrap::advance(
            &mut resources.write::<system::assembly::Jobs>().unwrap(), TaskId::new(4),
        ), Ok(Progress::Done));
        EFFECTS.with(|effects| {
            let effects = effects.borrow();
            assert_eq!((effects.unseals, effects.pushes, effects.ships, effects.narrows), (1, 1, 1, 1));
        });
    }
}

#[test]
fn expired_retry_does_not_publish_and_retains_the_root_for_release() {
    for boundary in 0..4 {
        reset();
        local();
        let mut jobs = make_jobs();
        EFFECTS.with(|effects| {
            let mut effects = effects.borrow_mut();
            effects.fail_unseal = boundary == 0;
            effects.fail_push = boundary == 1;
            effects.fail_ship = boundary == 2;
            effects.fail_narrow = boundary == 3;
        });
        assert_eq!(system::assembly::bootstrap::advance(&mut jobs, TaskId::new(4)), Ok(schedule::Progress::Pending));
        EFFECTS.with(|effects| {
            let mut effects = effects.borrow_mut();
            effects.fail_unseal = false;
            effects.fail_push = false;
            effects.fail_ship = false;
            effects.fail_narrow = false;
            effects.now = support::timing::BOOT_MS as u64 * 1_000_000;
        });
        assert_eq!(system::assembly::bootstrap::advance(&mut jobs, TaskId::new(4)), Err("bootstrap timeout"));
        EFFECTS.with(|effects| {
            let effects = effects.borrow();
            assert_eq!((effects.unseals, effects.pushes, effects.ships, effects.narrows),
                (usize::from(boundary > 0), usize::from(boundary > 1), usize::from(boundary > 2), 0));
        });
        system::assembly::bootstrap::release(&mut jobs).unwrap();
        assert!(jobs.launches[0].installation.bootstrap.is_none());
        if boundary > 0 { assert_same_handle(); }
        EFFECTS.with(|effects| assert_eq!(effects.borrow().releases, usize::from(boundary > 0)));
    }
}

#[test]
fn valid_carrier_returns_verified_creator_and_gateway() {
    reset();
    local();
    set_pie(PieKind::Hole, Permission::FETCH, TaskId::new(1), TaskId::new(1));
    set_pull(TaskId::new(1), TaskId::new(4), ROLE, VERSION, TaskId::new(1), Frame::LEN);
    let claimed = client::claim(Wait::POLL).unwrap();
    assert_eq!(claimed.provider, TaskId::new(1));
    assert_eq!(claimed.system, TaskId::new(4));
    set_pull(TaskId::new(2), TaskId::new(4), ROLE, VERSION, TaskId::new(1), Frame::LEN);
    assert_eq!(client::claim(Wait::POLL), Err(Fail::Source));
    set_pull(TaskId::new(1), TaskId::new(0), ROLE, VERSION, TaskId::new(1), Frame::LEN);
    assert_eq!(client::claim(Wait::POLL), Err(Fail::Source));
    EFFECTS.with(|effects| effects.borrow_mut().creator = TaskId::new(0));
    assert_eq!(client::claim(Wait::POLL), Err(Fail::Source));
}

#[test]
fn multiple_children_get_independent_carriers() {
    reset();
    local();
    let mut jobs = make_jobs();
    system::assembly::bootstrap::prepare(&mut jobs, TaskId::new(4)).unwrap();
    jobs.active = Some(system::assembly::Reference { task: TaskId::new(8) });
    system::assembly::bootstrap::prepare(&mut jobs, TaskId::new(4)).unwrap();
    EFFECTS.with(|effects| {
        let effects = effects.borrow();
        assert_eq!((effects.unseals, effects.pushes, effects.ships, effects.narrows), (2, 2, 2, 2));
        assert_eq!(effects.shipped_to, vec![TaskId::new(7), TaskId::new(8)]);
    });
}

#[test]
fn missing_reference_or_launch_does_not_unseal() {
    reset();
    local();
    let mut jobs = make_jobs();
    jobs.active = None;
    assert_eq!(
        system::assembly::bootstrap::prepare(&mut jobs, TaskId::new(4)),
        Err("bootstrap reference")
    );
    assert_eq!(EFFECTS.with(|effects| effects.borrow().unseals), 0);
    let mut jobs = make_jobs();
    jobs.launches.clear();
    assert_eq!(
        system::assembly::bootstrap::prepare(&mut jobs, TaskId::new(4)),
        Err("bootstrap launch")
    );
    assert_eq!(EFFECTS.with(|effects| effects.borrow().unseals), 0);
}

#[test]
fn client_rejects_wrong_source_short_frame_role_version() {
    reset();
    local();
    set_pie(PieKind::Hole, Permission::FETCH, TaskId::new(1), TaskId::new(1));
    set_pull(TaskId::new(1), TaskId::new(4), ROLE, VERSION, TaskId::new(2), Frame::LEN);
    assert_eq!(client::claim(Wait::POLL), Err(Fail::Source));
    set_pull(TaskId::new(1), TaskId::new(4), ROLE, VERSION, TaskId::new(1), 3);
    assert_eq!(client::claim(Wait::POLL), Err(Fail::Frame));
    set_pull(TaskId::new(1), TaskId::new(4), 9, VERSION, TaskId::new(1), Frame::LEN);
    assert_eq!(client::claim(Wait::POLL), Err(Fail::Frame));
    set_pull(TaskId::new(1), TaskId::new(4), ROLE, VERSION + 1, TaskId::new(1), Frame::LEN);
    assert_eq!(client::claim(Wait::POLL), Err(Fail::Frame));
}

#[test]
fn client_rejects_foreign_kind_permission_and_duplicates() {
    reset();
    local();
    set_pie(PieKind::Pole, Permission::FETCH, TaskId::new(1), TaskId::new(1));
    assert_eq!(client::claim(Wait::POLL), Err(Fail::Missing));
    set_pie(PieKind::Hole, Permission::STORE, TaskId::new(1), TaskId::new(1));
    assert_eq!(client::claim(Wait::POLL), Err(Fail::Missing));
    set_pie(PieKind::Hole, Permission::FETCH, TaskId::new(9), TaskId::new(1));
    assert_eq!(client::claim(Wait::POLL), Err(Fail::Missing));
    set_pie(PieKind::Hole, Permission::FETCH, TaskId::new(1), TaskId::new(9));
    assert_eq!(client::claim(Wait::POLL), Err(Fail::Missing));
    set_pie(PieKind::Hole, Permission::FETCH, TaskId::new(1), TaskId::new(1));
    set_pull(TaskId::new(1), TaskId::new(4), ROLE, VERSION, TaskId::new(1), Frame::LEN);
    EFFECTS.with(|effects| {
        let mut effects = effects.borrow_mut();
        let mut second = effects.pies[0].clone();
        second.token = PieToken::mint(22);
        effects.pies.push(second);
    });
    assert_eq!(client::claim(Wait::POLL), Err(Fail::Source));
}

#[test]
fn mark_is_registered_loader_channel() {
    assert_eq!(
        system_api::bootstrap::MARK.get(),
        system_api::loader::BOOTSTRAP.get()
    );
    let value = Frame {
        version: VERSION,
        role: ROLE,
        provider: TaskId::new(5),
        system: TaskId::new(4),
    };
    let mut bytes = <Frame as Message>::EMPTY;
    let len = value.store(&mut bytes).unwrap();
    assert_eq!(<Frame as Message>::fetch(&bytes[..len]), Some(value));
}
