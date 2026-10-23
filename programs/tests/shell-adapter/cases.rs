use super::adapt::controller::{Controller, RuntimeJob};
use super::adapt::ports::Buffer;
use super::core::JobId;
use super::native::objects::{LocalPort, Objects};
use super::*;
use system_api::control::frame::{OK, Said};
use system_api::identity::{PrincipalId, Subject};

const TASK: TaskId = TaskId::new(7);
const TEAM: TeamId = TeamId::new(3);

fn said() -> Said {
    Said {
        status: OK,
        a: system_api::control::State::Dead.code(),
        reason: 0,
        completed: true,
        task: TASK,
        team: TEAM,
    }
}

fn controller() -> Controller {
    let subject = Subject::new(PrincipalId::new(TaskId::new(1), 1), &[]).unwrap();
    Controller::new(
        shell_api::Catalogue { images: Vec::new() },
        pipe_client::Client,
        PieToken::NONE,
        PieToken::NONE,
        subject,
        TaskId::new(1),
        control::Face {
            task: TaskId::new(1),
        },
    )
}

fn root() -> lisp::RootValue {
    let engine = lisp::Engine::new(lisp::Limits::default()).unwrap();
    engine.nil()
}

#[test]
fn reclaim_pending_does_not_block_pumps_and_retries_after_rpc_timeout() {
    backend::reset();
    chrono::set(1_000);
    let mut controller = controller();
    let mut job = RuntimeJob::test_running(JobId(1), TASK, TEAM, root());
    job.test_push_pump(0);
    controller.test_push_job(job);
    let mut objects = Objects::new();
    objects.ports.push(Some(LocalPort {
        buffer: Buffer::Write { bytes: Vec::new() },
        closed: false,
        bound: None,
    }));
    backend::complete_team();
    backend::script_release(Ok(None));
    backend::script_release(Ok(None));
    backend::script_release(Err(system_api::control::Fail::Bad));
    backend::script_release(Ok(Some(said())));

    let before = backend::pump_steps();
    for _ in 0..2 {
        controller.advance(&mut objects);
    }
    assert!(
        backend::pump_steps() > before,
        "pumps must keep advancing while reclaim is pending"
    );
    assert_eq!(
        backend::release_begun(),
        1,
        "a pending release must not be replaced before its own deadline"
    );

    controller.advance(&mut objects);
    assert_eq!(backend::release_begun(), 1, "pending reply keeps the same request");
    controller.advance(&mut objects);
    assert_eq!(backend::release_begun(), 2, "timeout retries a fresh request");
    controller.advance(&mut objects);
    let job = controller.job(JobId(1)).unwrap();
    assert!(job.test_member_dead(0));
}

#[test]
fn terminal_job_still_reclaims_its_member() {
    backend::reset();
    chrono::set(1_000);
    let mut controller = controller();
    controller.test_push_job(RuntimeJob::test_running(JobId(2), TASK, TEAM, root()));
    let mut objects = Objects::new();
    backend::complete_team();
    backend::script_release(Ok(Some(said())));
    controller.cancel(JobId(2)).unwrap();
    for _ in 0..4 {
        controller.advance(&mut objects);
    }
    let job = controller.job(JobId(2)).unwrap();
    assert!(job.test_member_dead(0));
    assert!(job.model.status.terminal());
    assert_eq!(backend::release_begun(), 1);
}

#[test]
fn cancelling_job_with_an_owner_receipt_is_not_settled() {
    backend::reset();
    let mut controller = controller();
    let mut job = RuntimeJob::test_running(JobId(3), TASK, TEAM, root());
    job.model.fail();
    assert_eq!(job.model.status, core::JobStatus::Cancelling);
    controller.test_push_job(job);
    assert!(!controller.settled(), "terminal status is not receipt settlement");
}

#[test]
fn cancelling_job_with_an_owner_receipt_is_not_evicted() {
    let mut controller = controller();
    let mut job = RuntimeJob::test_running(JobId(3), TASK, TEAM, root());
    job.model.fail();
    assert_eq!(job.model.status, core::JobStatus::Cancelling);
    controller.test_push_job(job);
    controller.reclaim(&Objects::new());
    assert!(controller.job(JobId(3)).is_some(), "unreturned members were discarded");
}

#[test]
fn reclaim_pending_still_registers_pipeline_io_waits() {
    backend::reset();
    let mut controller = controller();
    let mut job = RuntimeJob::test_running(JobId(3), TASK, TEAM, root());
    job.test_push_pump(0);
    controller.test_push_job(job);
    let mut objects = Objects::new();
    objects.ports.push(Some(LocalPort {
        buffer: Buffer::Write { bytes: Vec::new() },
        closed: false,
        bound: None,
    }));
    backend::complete_team();
    controller.advance(&mut objects);
    let mut interests = adapt::waiting::Interests::new();
    controller.interests(&objects, &mut interests);
    assert!(interests.sources.contains(&ToleEventSource::Pole {
        token: PieToken::NONE, bit: pipe_api::DEMAND_BIT,
    }), "pending reclaim hides the pipeline's IO wakeup");
}

#[test]
fn consumed_team_does_not_fail_a_pending_release() {
    backend::reset();
    let mut controller = controller();
    controller.test_push_job(RuntimeJob::test_running(JobId(3), TASK, TEAM, root()));
    let mut objects = Objects::new();
    backend::complete_team();
    controller.advance(&mut objects);
    backend::consume_team();
    controller.advance(&mut objects);
    let job = controller.job(JobId(3)).unwrap();
    assert!(job.error.is_none(), "Control may Oust before its reply is observable");
    backend::script_release(Ok(Some(said())));
    controller.advance(&mut objects);
    assert!(controller.job(JobId(3)).unwrap().test_member_dead(0));
}
