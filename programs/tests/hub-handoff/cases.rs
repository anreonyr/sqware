use super::*;
use backend::Kind;
use env::{TaskId, TeamId};
use serve::execution::Executions;
use serve::peers::{ControlFace, Peers};
use service::hub::core::Ledger;
use system_api::control::{Fail, State, frame::Said};

const TASK: TaskId = TaskId::new(7);
const TASK2: TaskId = TaskId::new(8);
const TEAM: TeamId = TeamId::new(3);
const NOW: u64 = 1_000;

fn peers() -> Peers {
    Peers {
        control: ControlFace,
        coord: serve::peers::CoordFace,
    }
}

fn ok_said(task: TaskId, state: State) -> Said {
    Said {
        status: system_api::control::frame::OK,
        a: state.code(),
        reason: 0,
        completed: false,
        task,
        team: TEAM,
    }
}

fn fail_said(fail: Fail) -> Said {
    Said {
        status: system_api::control::frame::fail_to_code(Some(fail)),
        a: 0,
        reason: 0,
        completed: false,
        task: TASK,
        team: TEAM,
    }
}

fn ledger() -> Ledger {
    Ledger::new().with_hub(TaskId::new(1))
}

#[test]
fn admission_exists_before_embark_is_requested_and_lost_reply_does_not_duplicate() {
    let mut ledger = ledger();
    let mut executions = Executions::try_new().unwrap();
    let peers = peers();
    backend::reset();
    backend::script_inspect(Ok(Some(ok_said(TASK, State::Starting))));
    assert!(executions.receive(&ledger, TASK, NOW));

    executions.advance(&mut ledger, &peers, NOW);
    assert_eq!(backend::began(Kind::Inspect), 1);
    assert!(ledger.admission(TASK).is_none());

    executions.advance(&mut ledger, &peers, NOW);
    assert_eq!(backend::began(Kind::Embark), 1);
    assert_eq!(ledger.admissions().count(), 1);
    assert!(ledger.admission(TASK).is_some());

    backend::script_embark(Err(Fail::Bad));
    executions.advance(&mut ledger, &peers, NOW);
    executions.advance(&mut ledger, &peers, NOW);
    assert_eq!(backend::began(Kind::Embark), 2);
    assert_eq!(ledger.admissions().count(), 1);

    backend::script_embark(Ok(Some(ok_said(TASK, State::Starting))));
    executions.advance(&mut ledger, &peers, NOW);
    assert!(ledger.admission(TASK).is_some());
    assert!(!executions.has_return(TASK));
}

#[test]
fn stopping_state_is_not_admitted_and_gets_a_trusted_handoff() {
    let mut ledger = ledger();
    let mut executions = Executions::try_new().unwrap();
    let peers = peers();
    backend::reset();
    backend::script_inspect(Ok(Some(ok_said(TASK, State::Stopping))));
    assert!(executions.receive(&ledger, TASK, NOW));

    executions.advance(&mut ledger, &peers, NOW);
    executions.advance(&mut ledger, &peers, NOW);
    assert!(ledger.admission(TASK).is_none());
    assert!(executions.has_return(TASK));
}

#[test]
fn trusted_release_errors_and_timeouts_keep_the_same_obligation() {
    let mut ledger = ledger();
    ledger.admit(TASK, u64::MAX).unwrap();
    let mut executions = Executions::try_new().unwrap();
    let peers = peers();
    backend::reset();
    assert!(executions.target(&mut ledger, TASK));
    backend::script_release(Err(Fail::Bad));
    backend::script_release(Ok(Some(fail_said(Fail::Unknown))));
    backend::script_release(Ok(Some(fail_said(Fail::Denied))));
    backend::script_release(Ok(None));
    backend::script_release(Ok(Some(ok_said(TASK, State::Dead))));

    for _ in 0..5 {
        executions.advance(&mut ledger, &peers, NOW);
        assert!(executions.has_return(TASK));
        assert!(ledger.admission(TASK).is_some());
        assert!(!executions.idle(&ledger));
    }
    executions.advance(&mut ledger, &peers, NOW);
    assert!(!executions.has_return(TASK));
    assert!(ledger.admission(TASK).is_none());
    assert!(executions.idle(&ledger));
}

#[test]
fn target_without_a_launch_authenticates_owner_then_releases() {
    let mut ledger = ledger();
    let mut executions = Executions::try_new().unwrap();
    let peers = peers();
    backend::reset();
    assert!(executions.target(&mut ledger, TASK));
    backend::script_inspect(Ok(Some(ok_said(TASK, State::Starting))));
    backend::script_release(Ok(Some(ok_said(TASK, State::Dead))));

    executions.advance(&mut ledger, &peers, NOW);
    executions.advance(&mut ledger, &peers, NOW);
    executions.advance(&mut ledger, &peers, NOW);
    assert_eq!(backend::began(Kind::Release), 1);
    assert!(ledger.admission(TASK).is_none());
    assert!(!executions.has_return(TASK));
}

#[test]
fn foreign_owner_target_has_no_receipt_and_no_hanging_job() {
    let mut ledger = ledger();
    let mut executions = Executions::try_new().unwrap();
    let peers = peers();
    backend::reset();
    assert!(executions.target(&mut ledger, TASK));
    backend::script_inspect(Ok(Some(fail_said(Fail::Denied))));

    executions.advance(&mut ledger, &peers, NOW);
    executions.advance(&mut ledger, &peers, NOW);
    assert!(!executions.has_return(TASK));
    assert!(ledger.admission(TASK).is_none());
    assert!(executions.idle(&ledger));
}

#[test]
fn closing_binding_requests_release_before_hardware_finishes() {
    let mut ledger = ledger();
    ledger.admit(TASK, u64::MAX).unwrap();
    ledger.prepare_enrollment(TaskId::new(2), hub_api::Enroll {
        op: hub_api::DMA_ENROLL, key: 0, line: 7,
        name: "disk".into(), class: "virtio-blk".into(), back: env::PieToken::NONE,
    }).unwrap();
    let device = hub_api::DeviceId { source: TaskId::new(2), key: 0 };
    assert_eq!(ledger.advance_enrollment(device, &mut enrollment::Enrollment), Some(Ok(())));
    let binding = ledger.claim(core::ClaimRequest {
        from: TASK, id: 1, device,
        sensor: env::PieToken::mint(TASK.get()),
    }).unwrap();
    ledger.close(binding.serial, hub_api::Close::Normal).unwrap();
    assert_eq!(ledger.open_bindings_for(TASK), 1);
    ledger.settle();
    let mut executions = Executions::try_new().unwrap();
    let peers = peers();
    backend::reset();

    executions.settle(&mut ledger);
    assert!(executions.closed(&mut ledger, TASK));
    assert!(!executions.target_received(TASK));
    executions.advance(&mut ledger, &peers, NOW);
    assert_eq!(backend::began(Kind::Release), 1);
    assert!(executions.has_return(TASK));
    assert!(ledger.admission(TASK).is_some());
    assert!(!executions.target_received(TASK));
    assert_eq!(ledger.binding(binding.serial).unwrap().state, hub_api::BindingState::Closing);
    assert!(executions.target(&mut ledger, TASK));
    let mut crossed = 0;
    assert_eq!(ledger.stop_target(TASK, |_| crossed += 1), 1);
    assert_eq!(crossed, 1);
    assert!(executions.target_received(TASK));
    assert_eq!(ledger.binding(binding.serial).unwrap().state, hub_api::BindingState::Closing);
}

#[test]
fn full_return_queue_keeps_responsibility_until_a_slot_is_reliable() {
    let mut ledger = ledger();
    ledger.admit(TASK, u64::MAX).unwrap();
    ledger.admit(TASK2, u64::MAX).unwrap();
    let mut executions = Executions::with_capacity(1, 1).unwrap();
    let peers = peers();
    backend::reset();

    assert!(executions.target(&mut ledger, TASK));
    assert!(!executions.target(&mut ledger, TASK2));
    assert!(executions.has_return(TASK));
    assert!(!executions.has_return(TASK2));
    assert!(ledger.admission(TASK2).is_some());
    assert!(!ledger.admission(TASK2).unwrap().closed());
    assert!(!executions.target_received(TASK2));

    backend::script_release(Ok(Some(ok_said(TASK, State::Dead))));
    executions.advance(&mut ledger, &peers, NOW);
    executions.advance(&mut ledger, &peers, NOW);
    executions.advance(&mut ledger, &peers, NOW);
    assert!(!executions.has_return(TASK2));
    assert!(ledger.admission(TASK2).is_some());
    assert!(executions.target(&mut ledger, TASK2));
    assert!(executions.has_return(TASK2));
    assert!(executions.target_received(TASK2));
    assert!(ledger.admission(TASK2).unwrap().closed());
}

#[test]
fn closed_expired_and_global_settlement_returns_do_not_prove_cpu_stop() {
    backend::reset();
    let mut ledger = ledger();
    ledger.admit(TASK, NOW).unwrap();
    ledger.admit(TASK2, u64::MAX).unwrap();
    let mut executions = Executions::try_new().unwrap();
    executions.advance(&mut ledger, &peers(), NOW + 1);
    assert!(executions.has_return(TASK));
    assert!(!executions.target_received(TASK));
    assert!(executions.closed(&mut ledger, TASK2));
    assert!(!executions.target_received(TASK2));
    ledger.settle();
    executions.settle(&mut ledger);
    assert!(!executions.target_received(TASK));
    assert!(!executions.target_received(TASK2));
    assert!(executions.target(&mut ledger, TASK));
    assert!(executions.target_received(TASK));
    assert!(executions.closed(&mut ledger, TASK));
    executions.settle(&mut ledger);
    assert!(executions.target_received(TASK));
    assert!(!executions.target_received(TASK2));
}

#[test]
fn rejected_owner_discovery_keeps_target_proof_while_a_binding_is_open() {
    backend::reset();
    let mut ledger = ledger();
    let mut executions = Executions::try_new().unwrap();
    // Target arrival precedes admission discovery; its owner is still unconfirmed.
    assert!(executions.target(&mut ledger, TASK));
    ledger.admit(TASK, u64::MAX).unwrap();
    ledger.prepare_enrollment(TaskId::new(2), hub_api::Enroll {
        op: hub_api::DMA_ENROLL, key: 0, line: 7,
        name: "disk".into(), class: "virtio-blk".into(), back: env::PieToken::NONE,
    }).unwrap();
    let device = hub_api::DeviceId { source: TaskId::new(2), key: 0 };
    assert_eq!(ledger.advance_enrollment(device, &mut enrollment::Enrollment), Some(Ok(())));
    let binding = ledger.claim(core::ClaimRequest {
        from: TASK, id: 1, device,
        sensor: env::PieToken::mint(TASK.get()),
    }).unwrap();
    backend::script_inspect(Ok(Some(fail_said(Fail::Denied))));
    executions.advance(&mut ledger, &peers(), NOW);
    executions.advance(&mut ledger, &peers(), NOW);
    assert_eq!(backend::began(Kind::Release), 0);
    assert_eq!(ledger.open_bindings_for(TASK), 1);
    assert!(executions.has_return(TASK));
    assert!(executions.target_received(TASK));
    ledger.finish(binding.serial);
    backend::script_inspect(Ok(Some(fail_said(Fail::Denied))));
    executions.advance(&mut ledger, &peers(), NOW);
    executions.advance(&mut ledger, &peers(), NOW);
    assert!(!executions.has_return(TASK));
    assert!(!executions.target_received(TASK));
}

#[test]
fn duplicate_target_does_not_duplicate_the_return() {
    let mut ledger = ledger();
    ledger.admit(TASK, u64::MAX).unwrap();
    let mut executions = Executions::try_new().unwrap();
    let peers = peers();
    backend::reset();
    assert!(executions.target(&mut ledger, TASK));
    assert!(executions.target(&mut ledger, TASK));
    executions.advance(&mut ledger, &peers, NOW);
    assert_eq!(backend::began(Kind::Release), 1);
}

#[test]
fn launch_queue_is_bounded_without_consuming_a_report() {
    let ledger = ledger();
    let mut executions = Executions::with_capacity(1, 1).unwrap();
    assert!(executions.receive(&ledger, TASK, NOW));
    assert!(!executions.receive(&ledger, TASK2, NOW));
}

#[test]
fn queue_reservation_is_fallible() {
    assert!(Executions::with_capacity(usize::MAX, 1).is_err());
}

#[test]
fn authenticated_abort_survives_full_admission_and_return_queues() {
    backend::reset();
    let mut ledger = ledger();
    for task in 100..356 {
        ledger.admit(TaskId::new(task), u64::MAX).unwrap();
    }
    let mut executions = Executions::with_capacity(1, 1).unwrap();
    let peers = peers();
    assert!(executions.target(&mut ledger, TASK2));
    assert!(executions.receive(&ledger, TASK, NOW));
    backend::script_inspect(Ok(Some(ok_said(TASK, State::Debarked))));
    backend::script_inspect(Ok(Some(fail_said(Fail::Denied))));
    executions.advance(&mut ledger, &peers, NOW);
    executions.advance(&mut ledger, &peers, NOW);
    executions.advance(&mut ledger, &peers, NOW);
    assert!(executions.has_return(TASK), "authenticated receipt was dropped under backpressure");
    assert_eq!(backend::began(Kind::Embark), 0);
    backend::script_release(Ok(Some(ok_said(TASK, State::Dead))));
    executions.advance(&mut ledger, &peers, NOW);
    assert!(!executions.has_return(TASK));
    assert!(ledger.admission_closed(TASK));
}

#[test]
fn unseen_target_discovery_timeout_does_not_prove_absence() {
    backend::reset();
    let mut ledger = ledger();
    let mut executions = Executions::try_new().unwrap();
    let peers = peers();
    assert!(executions.target(&mut ledger, TASK));
    executions.advance(&mut ledger, &peers, NOW);
    let late = NOW + 4_000_000_000;
    executions.advance(&mut ledger, &peers, late);
    assert!(executions.has_return(TASK), "timeout erased an unconfirmed owner receipt");
    assert!(!executions.idle(&ledger));
    backend::script_inspect(Err(Fail::Bad));
    backend::script_inspect(Ok(Some(ok_said(TASK, State::Dead))));
    backend::script_release(Ok(Some(ok_said(TASK, State::Dead))));
    for _ in 0..4 {
        executions.advance(&mut ledger, &peers, late);
    }
    assert!(!executions.has_return(TASK));
    assert!(ledger.admission_closed(TASK));
}
