use crate::{PAGE_SIZE, Permission, TaskId};
use crate::attach_request::AttachRequest;
use crate::recovery_api::Fail;
use crate::recovery_core::Core;
use crate::state::{self, BAR, PCI, STATE};

fn request(serial: u64) -> AttachRequest {
    AttachRequest {
        serial,
        execution: TaskId::new(3),
        bar: BAR,
        pages: 4,
        common: 0,
        notify: PAGE_SIZE as u32,
        multiplier: 4,
        device: 3 * PAGE_SIZE as u32,
    }
}

#[test]
fn invalid_caps_are_rejected_before_mapping_or_granting() {
    state::reset();
    let mut core = Core::new(PCI);
    let mut invalid = request(1);
    invalid.common = invalid.notify;
    assert_eq!(core.attach(invalid), Err(Fail::Denied));
    STATE.with(|state| {
        let state = state.borrow();
        assert_eq!((state.opens, state.shuts, state.grants), (0, 0, 0));
        assert!(!state.mapped);
    });
}

#[test]
fn shape_failure_and_failed_shut_remain_owned_until_cleanup_succeeds() {
    state::reset();
    let mut core = Core::new(PCI);
    STATE.with(|state| state.borrow_mut().shut_fail = true);
    let mut invalid = request(1);
    invalid.pages = 5;
    assert_eq!(core.attach(invalid), Err(Fail::Unavailable));
    assert_eq!(core.stop(1), Err(Fail::Unavailable));
    assert_eq!(core.detach(1), Err(Fail::Unavailable));
    STATE.with(|state| {
        let state = state.borrow();
        assert_eq!((state.opens, state.shuts, state.grants), (1, 3, 0));
        assert!(state.mapped);
    });
    STATE.with(|state| state.borrow_mut().shut_fail = false);
    assert_eq!(core.stop(1), Ok(Some(0)));
    assert_eq!(core.detach(1), Ok(()));
    assert_eq!(core.attach(request(1)), Err(Fail::Conflict));
    STATE.with(|state| {
        let state = state.borrow();
        assert_eq!(state.shuts, 4);
        assert!(!state.mapped);
    });
    assert_eq!(core.attach(request(2)), Ok(()));
    STATE.with(|state| {
        let state = state.borrow();
        assert_eq!((state.opens, state.grants), (2, 1));
    });
}

#[test]
fn failed_child_grant_retries_same_mapping_and_conflicting_replay_is_rejected() {
    state::reset();
    let mut core = Core::new(PCI);
    STATE.with(|state| state.borrow_mut().grant_fail = true);
    assert_eq!(core.attach(request(1)), Err(Fail::Unavailable));
    assert_eq!(core.attached(), 1);
    assert_eq!(core.bar_grant(1), Err(Fail::Conflict));
    STATE.with(|state| state.borrow_mut().grant_fail = false);
    assert_eq!(core.attach(request(1)), Ok(()));
    assert_eq!(core.attach(request(1)), Ok(()));
    assert_eq!(core.bar_grant(1), Ok(crate::PieToken::mint(9)));
    assert_eq!(core.bar_grant(2), Err(Fail::Conflict));
    let mut conflict = request(1);
    conflict.multiplier += 1;
    assert_eq!(core.attach(conflict), Err(Fail::Conflict));
    let mut conflict = request(1);
    conflict.execution = TaskId::new(4);
    assert_eq!(core.attach(conflict), Err(Fail::Conflict));
    STATE.with(|state| {
        let state = state.borrow();
        assert_eq!((state.opens, state.shuts, state.grants), (1, 0, 1));
    });
    assert_eq!(core.stop(1), Ok(None));
    assert_eq!(core.stop(1), Ok(Some(0)));
    assert_eq!(core.detach(1), Ok(()));
    assert_eq!(core.attached(), 0);
    assert_eq!(core.bar_grant(1), Err(Fail::Conflict));
    assert_eq!(core.attach(request(2)), Ok(()));
    assert_eq!(core.bar_grant(2), Ok(crate::PieToken::mint(9)));
    STATE.with(|state| {
        let state = state.borrow();
        assert_eq!((state.opens, state.shuts, state.grants), (1, 0, 2));
        assert!(state.mapped, "baseline mapping persists across bindings");
    });
}

#[test]
fn queue_not_quiet_keeps_binding_and_blocks_detach_until_real_readback_changes() {
    state::reset();
    let mut core = Core::new(PCI);
    core.attach(request(1)).unwrap();
    STATE.with(|state| state.borrow_mut().memory[0x1c / 8] = 1 << 32);
    assert_eq!(core.stop(1), Ok(None));
    assert_eq!(core.stop(1), Err(Fail::Unavailable));
    assert_eq!(core.detach(1), Err(Fail::Conflict));
    assert_eq!(core.attached(), 1);
    STATE.with(|state| state.borrow_mut().memory[0x1c / 8] = 0);
    assert_eq!(core.stop(1), Ok(Some(0)));
    assert_eq!(core.detach(1), Ok(()));
    assert_eq!(core.attached(), 0);
}

#[test]
fn incorrect_source_or_permissions_cannot_create_recovery_mapping() {
    for wrong_source in [false, true] {
        state::reset();
        STATE.with(|state| {
            let mut state = state.borrow_mut();
            if wrong_source {
                state.info.vestor = TaskId::new(99);
            } else {
                state.info.permission.remove(Permission::VEST);
            }
        });
        let mut core = Core::new(PCI);
        assert_eq!(core.attach(request(1)), Err(Fail::Denied));
        STATE.with(|state| assert_eq!(state.borrow().opens, 0));
    }
}
