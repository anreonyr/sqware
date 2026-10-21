use env::{PieFail, PieToken, TaskId, TeamId};
use hub_api::Fail;
use system_api::control::Fail as ControlFail;

use crate::hub_core::{
    Execution, ExecutionEffects, ExecutionFailure as Failure, ExecutionStep as Step, Ledger,
};

const TASK: TaskId = TaskId::new(7);
const TEAM: TeamId = TeamId::new(9);

#[derive(Default)]
struct Effects {
    calls: Vec<Step>,
    fail: Option<Step>,
    absent: bool,
    exited: bool,
    oust_pending: bool,
    lost_embark_reply: bool,
    starts: usize,
    ousts: usize,
}

impl Effects {
    fn call(&mut self, step: Step) -> Result<(), Failure> {
        self.calls.push(step);
        if self.fail == Some(step) {
            self.fail = None;
            return Err(Failure::Pie(PieFail::OoM));
        }
        Ok(())
    }
}

impl ExecutionEffects for Effects {
    fn team(&mut self, task: TaskId) -> Result<Option<TeamId>, Failure> {
        assert_eq!(task, TASK);
        self.call(Step::Identify)?;
        Ok((!self.absent).then_some(TEAM))
    }

    fn embark(&mut self, task: TaskId) -> Result<(), Failure> {
        assert_eq!(task, TASK);
        self.call(Step::Embark)?;
        if self.starts == 0 {
            self.starts = 1;
        }
        if self.lost_embark_reply {
            self.lost_embark_reply = false;
            return Err(Failure::Control(ControlFail::Bad));
        }
        Ok(())
    }

    fn exited(&mut self, _: TaskId) -> Result<bool, Failure> {
        self.call(Step::Running)?;
        Ok(self.exited)
    }

    fn stop(&mut self, team: TeamId) -> Result<(), Failure> {
        assert_eq!(team, TEAM);
        self.call(Step::Stop)
    }

    fn oust(&mut self, team: TeamId) -> Result<bool, Failure> {
        assert_eq!(team, TEAM);
        self.call(Step::Oust)?;
        if self.oust_pending {
            return Ok(false);
        }
        self.ousts += 1;
        assert_eq!(self.ousts, 1, "never Oust a consumed receipt again");
        Ok(true)
    }

}

fn drive(execution: &mut Execution, effects: &mut Effects, target: Step) {
    for _ in 0..32 {
        if execution.step == target {
            return;
        }
        let _ = execution.advance(0, effects);
    }
    panic!("did not reach {target:?}: {:?}", execution.step);
}

#[test]
fn every_partial_startup_retries_without_recreating_completed_effects() {
    for fail in [Step::Identify, Step::Embark] {
        let mut execution = Execution::new(TASK, 100);
        let mut effects = Effects { fail: Some(fail), ..Effects::default() };
        drive(&mut execution, &mut effects, Step::Running);
        assert_eq!(execution.team, Some(TEAM));
        assert_eq!(effects.starts, 1);
        assert_eq!(execution.failure, Some(Failure::Pie(PieFail::OoM)));
        assert_eq!(effects.calls.iter().filter(|step| **step == fail).count(), 2);
        execution.retire();
        drive(&mut execution, &mut effects, Step::Done);
        assert!(execution.stopped());
        assert_eq!(effects.ousts, 1);
    }
}

#[test]
fn lost_embark_reply_retries_same_instance_without_starting_twice() {
    let mut execution = Execution::new(TASK, 100);
    let mut effects = Effects { lost_embark_reply: true, ..Effects::default() };
    drive(&mut execution, &mut effects, Step::Embark);
    assert_eq!(execution.advance(0, &mut effects), Err(Failure::Control(ControlFail::Bad)));
    assert_eq!(execution.step, Step::Embark);
    assert_eq!(effects.starts, 1);
    drive(&mut execution, &mut effects, Step::Running);
    assert_eq!(effects.starts, 1);
    assert_eq!(effects.calls.iter().filter(|step| **step == Step::Embark).count(), 2);
}

#[test]
fn timeout_at_every_startup_boundary_stops_verified_team_without_starting() {
    for boundary in [Step::Identify, Step::Embark] {
        let mut execution = Execution::new(TASK, 100);
        let mut effects = Effects::default();
        drive(&mut execution, &mut effects, boundary);
        execution.advance(100, &mut effects).unwrap();
        drive(&mut execution, &mut effects, Step::Done);
        assert_eq!(effects.starts, 0);
        assert!(execution.stopped());
        assert_eq!(effects.ousts, 1);
    }
}

#[test]
fn stop_confirmation_keeps_the_same_receipt_until_oust_completes() {
    let mut execution = Execution::new(TASK, 100);
    let mut effects = Effects::default();
    drive(&mut execution, &mut effects, Step::Running);
    execution.retire();
    drive(&mut execution, &mut effects, Step::Oust);
    effects.oust_pending = true;
    for _ in 0..3 {
        execution.retire();
        execution.advance(0, &mut effects).unwrap();
        assert!(!execution.stopped());
        assert_eq!(execution.team, Some(TEAM));
        assert_eq!(execution.step, Step::Oust);
        assert_eq!(effects.ousts, 0);
    }
    effects.oust_pending = false;
    execution.advance(0, &mut effects).unwrap();
    assert!(execution.stopped());
    execution.retire();
    assert_eq!(execution.step, Step::Done);
    assert_eq!(execution.team, Some(TEAM));
    drive(&mut execution, &mut effects, Step::Done);
    execution.advance(0, &mut effects).unwrap();
    assert_eq!(effects.ousts, 1);
}

#[test]
fn absent_or_foreign_instance_is_not_a_stop_confirmation() {
    let mut execution = Execution::new(TASK, 100);
    let mut effects = Effects { absent: true, ..Effects::default() };
    drive(&mut execution, &mut effects, Step::Done);
    assert!(!execution.stopped());
    assert_eq!(effects.calls, [Step::Identify]);
}

#[test]
fn main_exit_without_a_binding_still_retires_the_whole_team() {
    let mut execution = Execution::new(TASK, 100);
    let mut effects = Effects::default();
    drive(&mut execution, &mut effects, Step::Running);
    execution.advance(1000, &mut effects).unwrap();
    assert_eq!(execution.step, Step::Running, "running tasks do not inherit startup timeout");
    effects.exited = true;
    execution.advance(1000, &mut effects).unwrap();
    drive(&mut execution, &mut effects, Step::Done);
    assert_eq!(effects.ousts, 1);
}

#[test]
fn settle_counts_and_retires_launches_before_their_first_dma_claim() {
    let mut ledger = Ledger::new();
    ledger.launch(TASK, 100).unwrap();
    assert_eq!(ledger.settle(), 1);
    let mut effects = Effects::default();
    drive(ledger.execution(TASK).unwrap(), &mut effects, Step::Done);
    assert_eq!(effects.starts, 0);
    assert_eq!(ledger.settle(), 0);
    ledger.prune_executions();
    assert!(ledger.execution(TASK).is_none());
    assert_eq!(ledger.launch(TASK, 200), Err(Fail::Denied));
}

#[test]
fn settle_after_lost_embark_reply_stops_the_already_running_instance() {
    let mut ledger = Ledger::new();
    ledger.launch(TASK, 100).unwrap();
    let mut effects = Effects { lost_embark_reply: true, ..Effects::default() };
    let execution = ledger.execution(TASK).unwrap();
    drive(execution, &mut effects, Step::Embark);
    assert!(execution.advance(0, &mut effects).is_err());
    assert_eq!(effects.starts, 1);
    assert_eq!(ledger.settle(), 1);
    drive(ledger.execution(TASK).unwrap(), &mut effects, Step::Done);
    assert_eq!(ledger.settle(), 0);
    assert_eq!((effects.starts, effects.ousts), (1, 1));
}

#[test]
fn execution_capacity_is_bounded_and_full_does_not_create_a_record() {
    let mut ledger = Ledger::new();
    for task in 1..=256 {
        ledger.launch(TaskId::new(task), 100).unwrap();
    }
    assert_eq!(ledger.launch(TaskId::new(257), 100), Err(Fail::Full));
    assert!(ledger.execution(TaskId::new(257)).is_none());
    assert_eq!(ledger.launch(TASK, 200), Ok(()));
    assert_eq!(ledger.execution(TASK).unwrap().deadline, 100);
}

#[test]
fn failed_identity_query_after_timeout_keeps_retirement_intent() {
    let mut execution = Execution::new(TASK, 100);
    let mut effects = Effects { fail: Some(Step::Identify), ..Effects::default() };
    assert!(execution.advance(100, &mut effects).is_err());
    assert_eq!(execution.step, Step::Identify);
    drive(&mut execution, &mut effects, Step::Done);
    assert!(execution.stopped());
    assert_eq!((effects.starts, effects.ousts), (0, 1));
}

#[test]
fn failed_stop_or_oust_cannot_become_a_stop_confirmation() {
    for boundary in [Step::Stop, Step::Oust] {
        let mut execution = Execution::new(TASK, 100);
        let mut effects = Effects::default();
        drive(&mut execution, &mut effects, Step::Running);
        execution.retire();
        drive(&mut execution, &mut effects, boundary);
        effects.fail = Some(boundary);
        assert!(execution.advance(0, &mut effects).is_err());
        assert_eq!(execution.step, boundary);
        assert!(!execution.stopped());
        assert_eq!(effects.ousts, 0);
        drive(&mut execution, &mut effects, Step::Done);
        assert_eq!(effects.ousts, 1);
    }
}

#[test]
fn stopped_execution_receipt_is_retained_until_its_binding_is_closed() {
    use crate::hub_core::ClaimRequest;
    let mut ledger = Ledger::new();
    crate::enrollment::enroll(&mut ledger, &[8]);
    ledger.launch(TASK, 100).unwrap();
    let mut effects = Effects::default();
    drive(ledger.execution(TASK).unwrap(), &mut effects, Step::Running);
    let serial = ledger.claim(ClaimRequest {
        from: TASK,
        id: 1,
        requester: 8,
        sensor: PieToken::mint(5),
        recovery: TaskId::new(3),
    }).unwrap().serial;
    let execution = ledger.execution(TASK).unwrap();
    execution.retire();
    drive(execution, &mut effects, Step::Done);
    ledger.prune_executions();
    assert!(ledger.execution(TASK).unwrap().stopped());
    ledger.finish(serial);
    ledger.prune_executions();
    assert!(ledger.execution(TASK).is_none());
    assert_eq!(effects.ousts, 1);
}
