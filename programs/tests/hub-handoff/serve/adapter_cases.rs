use env::{PieToken, TaskId};
use hub_api::{BindingState, Close, DeviceId, DmaSaid, Failure, coordination::Wire};
use crate::{backend, coord_backend};
use crate::core::{ClaimRequest, Ledger};
use super::execution::Executions;
use super::peers::{ControlFace, CoordFace, Peers};
use super::coordination::advance;

const TASK: TaskId = TaskId::new(7);
const COORD: TaskId = TaskId::new(3);

fn ledger(devices: usize) -> Ledger {
    backend::reset();
    coord_backend::reset();
    let mut ledger = Ledger::new().with_hub(TaskId::new(1));
    ledger.admit(TASK, u64::MAX).unwrap();
    for key in 0..devices {
        ledger.prepare_enrollment(TaskId::new(3), hub_api::Enroll {
            op: hub_api::DMA_ENROLL, key: key as u64, line: 0,
            name: format!("disk{key}"), class: "disk".into(), back: PieToken::NONE,
        }).unwrap();
        let device = DeviceId { source: COORD, key: key as u64 };
        assert_eq!(ledger.advance_enrollment(device, &mut crate::enrollment::Enrollment), Some(Ok(())));
        ledger.claim(ClaimRequest {
            from: TASK, id: key as u64 + 1, device, sensor: PieToken::mint(7),
        }).unwrap();
    }
    ledger
}

fn peers() -> Peers { Peers { control: ControlFace, coord: CoordFace } }

#[test]
fn unrelated_binding_progresses_while_one_driver_reply_is_pending() {
    let mut ledger = ledger(2);
    let mut executions = Executions::try_new().unwrap();
    let mut pending = Vec::new();
    advance(&mut ledger, &peers(), &mut executions, &mut pending);
    assert_eq!(coord_backend::requests().len(), 2);
    coord_backend::script(2, Ok(Some(DmaSaid::bound(2, BindingState::Active))));
    advance(&mut ledger, &peers(), &mut executions, &mut pending);
    assert_eq!(ledger.binding(1).unwrap().state, BindingState::Preparing);
    assert_eq!(ledger.binding(2).unwrap().state, BindingState::Active);
    assert_eq!(pending.len(), 1);
    assert_eq!(coord_backend::requests().len(), 2);
}

#[test]
fn late_prepare_cannot_reactivate_closing_and_release_waits_for_actual_target() {
    for observed in [BindingState::Preparing, BindingState::Active, BindingState::Closing] {
    let mut ledger = ledger(1);
    let mut executions = Executions::try_new().unwrap();
    let mut pending = Vec::new();
    advance(&mut ledger, &peers(), &mut executions, &mut pending);
    ledger.settle();
    executions.settle(&mut ledger);
    coord_backend::script(1, Ok(Some(DmaSaid::bound(1, observed))));
    advance(&mut ledger, &peers(), &mut executions, &mut pending);
    assert_eq!(ledger.binding(1).unwrap().state, BindingState::Closing);
    assert_eq!(ledger.binding(1).unwrap().failure, None, "late Prepare is benign");
    assert_eq!(coord_backend::requests().len(), 1);
    assert!(executions.has_return(TASK));
    assert!(!executions.target_received(TASK));
    executions.advance(&mut ledger, &peers(), 1000);
    assert_eq!(backend::began(backend::Kind::Release), 1);
    assert!(executions.target(&mut ledger, TASK));
    advance(&mut ledger, &peers(), &mut executions, &mut pending);
    assert!(matches!(coord_backend::requests()[1], Wire::Release(_)));
    coord_backend::script(1, Ok(Some(DmaSaid::bound(1, BindingState::Closed))));
    advance(&mut ledger, &peers(), &mut executions, &mut pending);
    assert_eq!(ledger.binding(1).unwrap().state, BindingState::Closed);
    assert!(pending.is_empty());
    assert_eq!(coord_backend::requests().len(), 2);
    }
}

#[test]
fn rpc_failure_retries_same_binding_and_first_failure_survives_later_replies() {
    let mut ledger = ledger(1);
    let mut executions = Executions::try_new().unwrap();
    let mut pending = Vec::new();
    advance(&mut ledger, &peers(), &mut executions, &mut pending);
    coord_backend::script(1, Err(system_api::control::Fail::Bad));
    advance(&mut ledger, &peers(), &mut executions, &mut pending);
    let first = ledger.binding(1).unwrap().failure.unwrap();
    assert_eq!(coord_backend::requests()[0], coord_backend::requests()[1]);
    let later = Failure { party: 4, op: 9, code: 3 };
    coord_backend::script(1, Ok(Some(DmaSaid::progress(1, BindingState::Active, Some(later)))));
    advance(&mut ledger, &peers(), &mut executions, &mut pending);
    assert_eq!(ledger.binding(1).unwrap().state, BindingState::Active);
    assert_eq!(ledger.binding(1).unwrap().failure, Some(first));
    assert!(pending.is_empty());
}

#[test]
fn foreign_serial_and_wrong_state_cannot_finish_a_binding() {
    let mut ledger = ledger(1);
    let mut executions = Executions::try_new().unwrap();
    let mut pending = Vec::new();
    advance(&mut ledger, &peers(), &mut executions, &mut pending);
    for said in [DmaSaid::bound(9, BindingState::Active), DmaSaid::bound(1, BindingState::Closed)] {
        coord_backend::script(1, Ok(Some(said)));
        advance(&mut ledger, &peers(), &mut executions, &mut pending);
        assert_eq!(ledger.binding(1).unwrap().state, BindingState::Preparing);
        assert_eq!(pending.len(), 1);
    }
    ledger.close(1, Close::Fault).unwrap();
    coord_backend::script(1, Ok(Some(DmaSaid::bound(1, BindingState::Active))));
    advance(&mut ledger, &peers(), &mut executions, &mut pending);
    assert_eq!(ledger.binding(1).unwrap().state, BindingState::Closing);
    assert!(!coord_backend::requests().iter().any(|wire| matches!(wire, Wire::Release(_))));
}

#[test]
fn driver_capacity_retry_does_not_displace_the_first_actual_failure() {
    let mut ledger = ledger(1);
    let mut executions = Executions::try_new().unwrap();
    let mut pending = Vec::new();
    advance(&mut ledger, &peers(), &mut executions, &mut pending);
    coord_backend::script(1, Ok(Some(DmaSaid::of(hub_api::fail_to_code(Some(hub_api::Fail::Full))))));
    advance(&mut ledger, &peers(), &mut executions, &mut pending);
    assert_eq!(ledger.binding(1).unwrap().failure, None);
    assert_eq!(ledger.binding(1).unwrap().state, BindingState::Preparing);
    let actual = Failure { party: hub_api::PARTY_IOMMU, op: 2, code: 4 };
    coord_backend::script(1, Ok(Some(DmaSaid::progress(1, BindingState::Preparing, Some(actual)))));
    advance(&mut ledger, &peers(), &mut executions, &mut pending);
    assert_eq!(ledger.binding(1).unwrap().failure, Some(actual));
}

#[test]
fn a_full_set_of_pending_bindings_stays_bounded_without_failing() {
    let mut ledger = ledger(hub_api::INVENTORY_MAX);
    let mut executions = Executions::try_new().unwrap();
    let mut pending = Vec::new();
    advance(&mut ledger, &peers(), &mut executions, &mut pending);
    assert_eq!(pending.len(), hub_api::INVENTORY_MAX);
    for _ in 0..3 { advance(&mut ledger, &peers(), &mut executions, &mut pending); }
    assert_eq!(pending.len(), hub_api::INVENTORY_MAX);
    assert_eq!(coord_backend::requests().len(), hub_api::INVENTORY_MAX);
    assert!(ledger.open_bindings_all().all(|binding| binding.failure.is_none()));
}

#[test]
fn a_matching_key_from_another_source_never_reaches_this_driver() {
    let mut ledger = ledger(1);
    let foreign = DeviceId { source: TaskId::new(4), key: 0 };
    ledger.prepare_enrollment(foreign.source, hub_api::Enroll {
        op: hub_api::DMA_ENROLL, key: foreign.key, line: 0,
        name: "foreign".into(), class: "disk".into(), back: PieToken::NONE,
    }).unwrap();
    assert_eq!(ledger.advance_enrollment(foreign, &mut crate::enrollment::Enrollment), Some(Ok(())));
    let id = ledger.claim(ClaimRequest {
        from: TASK, id: 2, device: foreign, sensor: PieToken::mint(7),
    }).unwrap();
    let mut executions = Executions::try_new().unwrap();
    let mut pending = Vec::new();
    advance(&mut ledger, &peers(), &mut executions, &mut pending);
    assert_eq!(coord_backend::requests().len(), 1);
    assert!(matches!(coord_backend::requests()[0], Wire::Prepare(frame) if frame.serial == 1 && frame.key == 0));
    assert_eq!(ledger.binding(id.serial).unwrap().state, BindingState::Preparing);
    assert_eq!(ledger.binding(id.serial).unwrap().failure, Some(Failure {
        party: hub_api::PARTY_HUB, op: hub_api::coordination::PREPARE,
        code: hub_api::fail_to_code(Some(hub_api::Fail::Denied)),
    }));
    assert_eq!(ledger.open_bindings(foreign), 1);
}
