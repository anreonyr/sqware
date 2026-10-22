#![allow(dead_code)]
extern crate alloc;
extern crate env as abi_env;
extern crate lisp;
extern crate pipe_api;
extern crate shell_api;
extern crate stream;
extern crate system_api;

extern crate self as env;
extern crate self as ipc;
extern crate self as pipe_client;
extern crate self as system_client;

use alloc::collections::VecDeque;

pub use abi_env::{
    ExitCause, JoinReply, PieToken, Reason, TaskExit, TaskId, TeamId, ToleEventSource, UnitFail,
    UnitTarget, Wait,
};

pub mod backend {
    use super::*;
    use ::core::cell::RefCell;

    #[derive(Default)]
    struct Backend {
        team_complete: bool,
        team_consumed: bool,
        release_begun: usize,
        pump_steps: usize,
        release: VecDeque<Result<Option<system_api::control::frame::Said>, system_api::control::Fail>>,
    }

    thread_local! {
        static BACKEND: RefCell<Backend> = RefCell::new(Backend::default());
    }

    pub fn reset() {
        BACKEND.with(|backend| *backend.borrow_mut() = Backend::default());
    }

    pub fn complete_team() {
        BACKEND.with(|backend| backend.borrow_mut().team_complete = true);
    }

    pub fn team_complete() -> bool {
        BACKEND.with(|backend| backend.borrow().team_complete)
    }

    pub fn consume_team() {
        BACKEND.with(|backend| backend.borrow_mut().team_consumed = true);
    }

    pub fn team_consumed() -> bool {
        BACKEND.with(|backend| backend.borrow().team_consumed)
    }

    pub fn release_begun() -> usize {
        BACKEND.with(|backend| backend.borrow().release_begun)
    }

    pub fn pump_steps() -> usize {
        BACKEND.with(|backend| backend.borrow().pump_steps)
    }

    pub fn pump_step() {
        BACKEND.with(|backend| backend.borrow_mut().pump_steps += 1);
    }

    pub fn script_release(
        reply: Result<Option<system_api::control::frame::Said>, system_api::control::Fail>,
    ) {
        BACKEND.with(|backend| backend.borrow_mut().release.push_back(reply));
    }

    pub fn begin_release() {
        BACKEND.with(|backend| backend.borrow_mut().release_begun += 1);
    }

    pub fn poll_release() -> Result<Option<system_api::control::frame::Said>, system_api::control::Fail>
    {
        BACKEND.with(|backend| {
            backend
                .borrow_mut()
                .release
                .pop_front()
                .unwrap_or(Ok(None))
        })
    }
}

pub mod chrono {
    use core::cell::Cell;
    thread_local! {
        static NOW: Cell<u64> = const { Cell::new(1_000) };
    }
    pub fn clock() -> u64 {
        NOW.with(|now| now.get())
    }
    pub fn set(value: u64) {
        NOW.with(|now| now.set(value));
    }
}

pub mod unit {
    use super::*;

    #[derive(Debug)]
    pub struct Error {
        pub source: UnitFail,
    }

    pub fn self_id() -> TaskId {
        TaskId::new(1)
    }

    pub fn join(
        _: UnitTarget,
        _: Wait,
        receive: bool,
    ) -> Result<JoinReply, Error> {
        if crate::backend::team_consumed() {
            return Err(Error { source: UnitFail::Denied });
        }
        if receive {
            return Ok(JoinReply::Pending);
        }
        if crate::backend::team_complete() {
            Ok(JoinReply::Complete)
        } else {
            Ok(JoinReply::Pending)
        }
    }

    pub fn slay_team(_: TeamId) -> Result<(), Error> {
        Ok(())
    }

    pub fn embark_task(_: TaskId) -> Result<(), Error> {
        Ok(())
    }

    pub fn embark_team(_: TeamId) -> Result<(), Error> {
        Ok(())
    }

    pub fn debark_team(_: TeamId) -> Result<(), Error> {
        Ok(())
    }
}

pub mod rpc {
    pub mod request {
        use ::core::marker::PhantomData;

        pub trait Reply {
            type In;
            type Fail;
            fn poll() -> Result<Option<Self::In>, Self::Fail>;
        }

        impl Reply for pipe_api::Call {
            type In = pipe_api::Reply;
            type Fail = pipe_api::Fail;
            fn poll() -> Result<Option<Self::In>, Self::Fail> {
                Ok(None)
            }
        }

        impl Reply for system_api::control::Call {
            type In = system_api::control::frame::Said;
            type Fail = system_api::control::Fail;
            fn poll() -> Result<Option<Self::In>, Self::Fail> {
                crate::backend::poll_release()
            }
        }

        pub struct Pending<C: Reply> {
            _contract: PhantomData<C>,
        }

        impl<C: Reply> Pending<C> {
            pub(crate) fn new() -> Self {
                Self {
                    _contract: PhantomData,
                }
            }

            pub fn poll(&mut self) -> Result<Option<C::In>, C::Fail> {
                C::poll()
            }

            pub fn source(&self) -> crate::ToleEventSource {
                crate::ToleEventSource::Join {
                    target: crate::unit::self_id().into(),
                }
            }

            pub fn remaining(&self) -> crate::Wait {
                crate::Wait::AtMost(1)
            }
        }
    }
}

pub mod control {
    use super::*;
    use system_api::control::{Call, Fail, frame::Said};

    pub struct Face {
        pub task: TaskId,
    }

    pub struct Instance {
        pub task: TaskId,
    }

    impl Face {
        pub fn instance(&self, task: TaskId) -> Instance {
            Instance { task }
        }
    }

    impl Instance {
        pub fn begin_release(
            &self,
            _: Wait,
        ) -> Result<crate::rpc::request::Pending<Call>, Fail> {
            crate::backend::begin_release();
            Ok(crate::rpc::request::Pending::new())
        }

        pub fn release_reply(&self, said: Said) -> Result<(), Fail> {
            match system_api::control::frame::code_to_fail(said.status) {
                None if said.status == system_api::control::frame::OK => Ok(()),
                Some(fail) => Err(fail),
                None => Err(Fail::Bad),
            }
        }
    }
}

pub use pipe_api::Direction;

pub struct Client;

pub struct Creating;

pub struct Pipe {
    pub id: u64,
    pub capacity: usize,
}

pub struct Port;

impl Port {
    pub fn token(&self) -> PieToken {
        PieToken::NONE
    }
    pub fn prepare_wait(&self) -> Result<bool, pipe_api::Fail> {
        Ok(false)
    }
    pub fn source(&self) -> ToleEventSource {
        ToleEventSource::Pole {
            token: PieToken::NONE,
            bit: pipe_api::DEMAND_BIT,
        }
    }
}

impl Client {
    pub fn begin_create(&self, _: usize, _: Wait) -> Result<Creating, pipe_api::Fail> {
        Ok(Creating)
    }

    pub fn begin_bind(
        &self,
        _: &Pipe,
        _: Direction,
        _: TaskId,
        _: Wait,
    ) -> Result<crate::rpc::request::Pending<pipe_api::Call>, pipe_api::Fail> {
        Ok(crate::rpc::request::Pending::new())
    }

    pub fn import(&self, _: pipe_api::Endpoint, _: Wait) -> Result<Port, pipe_api::Fail> {
        Ok(Port)
    }

    pub fn begin_release(
        &self,
        _: &Pipe,
        _: Wait,
    ) -> Result<crate::rpc::request::Pending<pipe_api::Call>, pipe_api::Fail> {
        Ok(crate::rpc::request::Pending::new())
    }
}

impl Creating {
    pub fn poll(&mut self) -> Result<Option<Pipe>, pipe_api::Fail> {
        Ok(None)
    }

    pub fn source(&self) -> ToleEventSource {
        ToleEventSource::Join {
            target: TaskId::new(1).into(),
        }
    }

    pub fn remaining(&self) -> Wait {
        Wait::AtMost(1)
    }
}

#[path = "../../src/user/shell/core/mod.rs"]
pub mod core;

pub mod native {
    pub mod objects {
        use super::super::adapt::ports::Buffer;
        use super::super::core::JobId;

        pub struct LocalPort {
            pub buffer: Buffer,
            pub closed: bool,
            pub bound: Option<JobId>,
        }

        pub struct Objects {
            pub used: usize,
            pub ports: Vec<Option<LocalPort>>,
            pub jobs: Vec<JobId>,
        }

        impl Objects {
            pub fn new() -> Self {
                Self {
                    used: 0,
                    ports: Vec::new(),
                    jobs: Vec::new(),
                }
            }

            pub fn used(&self) -> usize {
                self.used
            }

            pub fn port_mut(&mut self, id: u64) -> Option<&mut LocalPort> {
                self.ports.get_mut(id as usize).and_then(Option::as_mut)
            }

            pub fn port(&self, id: u64) -> Option<&LocalPort> {
                self.ports.get(id as usize).and_then(Option::as_ref)
            }

            pub fn has_job(&self, id: JobId) -> bool {
                self.jobs.contains(&id)
            }
        }
    }
}

pub mod adapt;

#[cfg(test)]
mod tests {
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
}
