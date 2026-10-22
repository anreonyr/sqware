//! Hub admission and target-retirement responsibilities.
//!
//! These replace the old Hub execution stage machine. Hub records only launch identity,
//! claim watermark, deadline and closed history; Control owns running/death state.

use env::{PieToken, TaskId};
use hub_api::{BindingState, Fail};

use crate::hub_core::{ClaimRequest, Ledger, Step};

const HUB: usize = 1;
const RECOVERY: usize = 3;
const TASK: TaskId = TaskId::new(9);

fn ledger() -> Ledger {
    let mut ledger = Ledger::new().with_hub(TaskId::new(HUB));
    crate::enrollment::enroll(&mut ledger, &[0]);
    ledger
}

fn admit(ledger: &mut Ledger, task: TaskId, deadline: u64) {
    ledger.admit(task, deadline).unwrap();
}

fn claim_at(ledger: &mut Ledger, task: TaskId, id: u64) -> Result<u64, Fail> {
    ledger
        .claim(ClaimRequest {
            from: task,
            id,
            requester: 0,
            sensor: PieToken::mint(task.get()),
            recovery: TaskId::new(RECOVERY),
        })
        .map(|binding| binding.serial)
}

#[test]
fn launch_replay_keeps_identity_and_deadline_without_a_second_record() {
    let mut ledger = ledger();
    admit(&mut ledger, TASK, 100);
    admit(&mut ledger, TASK, 200);
    assert_eq!(ledger.admissions().count(), 1);
    assert_eq!(ledger.admission(TASK).unwrap().deadline(), 100);
    assert!(ledger.admission(TASK).unwrap().claimable());
}

#[test]
fn claim_requires_an_open_admission_and_advances_the_watermark_once() {
    let mut ledger = ledger();
    assert_eq!(claim_at(&mut ledger, TASK, 1), Err(Fail::Denied));
    admit(&mut ledger, TASK, u64::MAX);
    assert_eq!(claim_at(&mut ledger, TASK, 1), Ok(1));
    assert_eq!(claim_at(&mut ledger, TASK, 1), Ok(1));
    assert_eq!(claim_at(&mut ledger, TASK, 0), Err(Fail::Bad));
    assert_eq!(claim_at(&mut ledger, TASK, 2), Err(Fail::Taken));
    // A denied claim does not advance the watermark or create a binding.
    assert_eq!(ledger.admission(TASK).unwrap().claim_high(), 1);
    assert_eq!(ledger.open_bindings_all().count(), 1);
}

#[test]
fn expired_launch_aborts_before_it_can_claim() {
    let mut ledger = ledger();
    admit(&mut ledger, TASK, 100);
    let mut expired = None;
    ledger.expire_admissions(100, |task| expired = Some(task));
    assert_eq!(expired, Some(TASK));
    assert!(ledger.admission(TASK).unwrap().closed());
    assert_eq!(claim_at(&mut ledger, TASK, 1), Err(Fail::Denied));
    assert_eq!(ledger.open_bindings_all().count(), 0);
}

#[test]
fn failed_or_foreign_launch_records_nothing() {
    let mut ledger = ledger();
    assert_eq!(ledger.admit(TaskId::new(0), 100), Err(Fail::Denied));
    admit(&mut ledger, TASK, 100);
    assert_eq!(claim_at(&mut ledger, TaskId::new(10), 1), Err(Fail::Denied));
    assert!(ledger.admission(TaskId::new(10)).is_none());
    assert!(ledger.admission(TASK).is_some());
}

#[test]
fn target_proof_advances_only_its_bound_binding_and_never_repeats_completed_steps() {
    let mut ledger = ledger();
    admit(&mut ledger, TASK, u64::MAX);
    let serial = claim_at(&mut ledger, TASK, 1).unwrap();
    ledger.close(serial, hub_api::Close::Normal).unwrap();
    assert_eq!(ledger.binding(serial).unwrap().step, Step::StopCpu);

    assert_eq!(ledger.stop_target(TASK), 1);
    assert_eq!(ledger.binding(serial).unwrap().step, Step::Invalidate);

    // Repeating the same proof must not replay an already completed transition.
    ledger.binding_mut(serial).unwrap().step = Step::Disable;
    assert_eq!(ledger.stop_target(TASK), 1);
    assert_eq!(ledger.binding(serial).unwrap().step, Step::Disable);
    assert!(ledger.admission(TASK).unwrap().closed());
}

#[test]
fn target_without_a_binding_is_resource_done_without_faking_lifecycle() {
    let mut ledger = ledger();
    admit(&mut ledger, TASK, 100);
    assert_eq!(ledger.stop_target(TASK), 0);
    assert_eq!(ledger.stop_target(TaskId::new(44)), 0);
    assert!(ledger.admission(TaskId::new(44)).is_none());
    assert!(ledger.admission(TASK).unwrap().closed());
}

#[test]
fn settle_retires_unbound_launches_and_keeps_the_receipt_until_release() {
    let mut ledger = ledger();
    admit(&mut ledger, TASK, 100);
    assert_eq!(ledger.settle(), 1);
    assert!(ledger.admission(TASK).unwrap().closed());
    assert_eq!(claim_at(&mut ledger, TASK, 1), Err(Fail::Denied));
    assert_eq!(ledger.settle(), 1);
    ledger.released(TASK);
    assert_eq!(ledger.settle(), 0);
    assert!(ledger.admission(TASK).is_none());
    assert_eq!(ledger.admit(TASK, 200), Err(Fail::Denied));
}

#[test]
fn target_proof_does_not_advance_another_executions_binding() {
    let other = TaskId::new(10);
    let mut ledger = Ledger::new().with_hub(TaskId::new(HUB));
    crate::enrollment::enroll(&mut ledger, &[0, 1]);
    admit(&mut ledger, TASK, u64::MAX);
    admit(&mut ledger, other, u64::MAX);
    let mine = ledger
        .claim(ClaimRequest {
            from: TASK,
            id: 1,
            requester: 0,
            sensor: PieToken::mint(TASK.get()),
            recovery: TaskId::new(RECOVERY),
        })
        .unwrap()
        .serial;
    let theirs = ledger
        .claim(ClaimRequest {
            from: other,
            id: 1,
            requester: 1,
            sensor: PieToken::mint(other.get()),
            recovery: TaskId::new(RECOVERY),
        })
        .unwrap()
        .serial;
    ledger.close(mine, hub_api::Close::Normal).unwrap();
    ledger.close(theirs, hub_api::Close::Normal).unwrap();

    assert_eq!(ledger.stop_target(TASK), 1);
    assert_eq!(ledger.binding(mine).unwrap().step, Step::Invalidate);
    assert_eq!(ledger.binding(theirs).unwrap().step, Step::StopCpu);
    assert!(ledger.admission(TASK).unwrap().closed());
    assert!(!ledger.admission(other).unwrap().closed());
}

#[test]
fn settle_closes_bindings_but_does_not_pretend_the_cpu_stopped() {
    let mut ledger = ledger();
    admit(&mut ledger, TASK, u64::MAX);
    let serial = claim_at(&mut ledger, TASK, 1).unwrap();
    assert_eq!(ledger.settle(), 2);
    assert_eq!(ledger.binding(serial).unwrap().state, BindingState::Closing);
    assert_eq!(ledger.binding(serial).unwrap().step, Step::StopCpu);
    assert_eq!(claim_at(&mut ledger, TaskId::new(10), 1), Err(Fail::Denied));
    assert_eq!(ledger.stop_target(TASK), 1);
    assert_eq!(ledger.binding(serial).unwrap().step, Step::Invalidate);
    ledger.finish(serial);
    ledger.released(TASK);
    assert_eq!(ledger.settle(), 0);
}

#[test]
fn closed_launch_history_is_bounded_and_stops_a_late_replay() {
    let mut ledger = ledger();
    for task in 1..=40 {
        let task = TaskId::new(task);
        admit(&mut ledger, task, 100);
        ledger.released(task);
    }
    assert!(ledger.admission(TaskId::new(40)).is_none());
    assert_eq!(ledger.admit(TaskId::new(40), 100), Err(Fail::Denied));
    // The bounded history is not a second lifecycle ledger.
    assert!(ledger.closed_admission_count() <= 32);
}

#[test]
fn admission_capacity_is_bounded_and_full_creates_no_record() {
    let mut ledger = Ledger::new();
    for task in 1..=256 {
        admit(&mut ledger, TaskId::new(task), 100);
    }
    assert_eq!(ledger.admit(TaskId::new(257), 100), Err(Fail::Full));
    assert!(ledger.admission(TaskId::new(257)).is_none());
    assert_eq!(ledger.admissions().count(), 256);
}
