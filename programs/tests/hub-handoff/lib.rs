#![allow(dead_code)]
extern crate alloc;
extern crate env;
extern crate hub_api;
extern crate system_api;

extern crate self as ipc;
extern crate self as resource;

use alloc::collections::VecDeque;
use ::core::cell::RefCell;
use env::{TaskId, TeamId};
use system_api::control::{Fail, State, frame::Said};

pub mod backend {
    use super::*;

    #[derive(Clone, Copy, PartialEq, Eq, Debug)]
    pub enum Kind {
        Inspect,
        Embark,
        Release,
    }

    #[derive(Default)]
    struct Backend {
        inspect: VecDeque<Result<Option<Said>, Fail>>,
        embark: VecDeque<Result<Option<Said>, Fail>>,
        release: VecDeque<Result<Option<Said>, Fail>>,
        began: alloc::vec::Vec<Kind>,
    }

    thread_local! {
        static BACKEND: RefCell<Backend> = RefCell::new(Backend::default());
    }

    pub fn reset() {
        BACKEND.with(|backend| *backend.borrow_mut() = Backend::default());
    }

    pub fn script_inspect(reply: Result<Option<Said>, Fail>) {
        BACKEND.with(|backend| backend.borrow_mut().inspect.push_back(reply));
    }

    pub fn script_embark(reply: Result<Option<Said>, Fail>) {
        BACKEND.with(|backend| backend.borrow_mut().embark.push_back(reply));
    }

    pub fn script_release(reply: Result<Option<Said>, Fail>) {
        BACKEND.with(|backend| backend.borrow_mut().release.push_back(reply));
    }

    pub fn began(kind: Kind) -> usize {
        BACKEND.with(|backend| backend.borrow().began.iter().filter(|seen| **seen == kind).count())
    }

    pub fn begin(kind: Kind) {
        BACKEND.with(|backend| backend.borrow_mut().began.push(kind));
    }

    pub fn poll(kind: Kind) -> Result<Option<Said>, Fail> {
        BACKEND.with(|backend| {
            let mut backend = backend.borrow_mut();
            let queue = match kind {
                Kind::Inspect => &mut backend.inspect,
                Kind::Embark => &mut backend.embark,
                Kind::Release => &mut backend.release,
            };
            queue.pop_front().unwrap_or(Ok(None))
        })
    }

    pub fn reply(said: Said) -> Result<(), Fail> {
        match system_api::control::frame::code_to_fail(said.status) {
            None if said.status == system_api::control::frame::OK => Ok(()),
            Some(fail) => Err(fail),
            None => Err(Fail::Bad),
        }
    }
}

pub mod rpc {
    pub mod request {
        use ::core::marker::PhantomData;
        use system_api::control::{Fail, frame::Said};

        pub struct Pending<C> {
            kind: crate::backend::Kind,
            _contract: PhantomData<C>,
        }

        impl<C> Pending<C> {
            pub(crate) fn new(kind: crate::backend::Kind) -> Self {
                Self {
                    kind,
                    _contract: PhantomData,
                }
            }

            pub fn poll(&mut self) -> Result<Option<Said>, Fail> {
                crate::backend::poll(self.kind)
            }
        }
    }
}

pub mod raw {
    use env::{Mark, PieKind, PieToken, Wait};

    pub struct Info {
        pub alive: bool,
        pub kind: PieKind,
        pub mark: Mark,
        pub token: PieToken,
    }

    pub fn pies() -> impl Iterator<Item = Info> {
        ::core::iter::empty()
    }

    pub struct Hole;

    impl Hole {
        pub fn from_raw(_: PieToken) -> Self {
            Hole
        }

        pub fn pull(&self, _: &mut [u8], _: Wait) -> Result<(usize, PieToken), ()> {
            Err(())
        }
    }
}

#[path = "../../src/service/hub/core/mod.rs"]
pub mod core;

pub mod service {
    pub mod hub {
        pub use crate::core;
    }
}

pub mod serve;
mod enrollment;

pub mod debug {
    pub fn put(_: &str) {}
}

#[cfg(test)]
use backend::Kind;
use service::hub::core::Ledger;
#[cfg(test)]
use serve::execution::Executions;
use serve::peers::{ControlFace, Peers};

const TASK: TaskId = TaskId::new(7);
const TASK2: TaskId = TaskId::new(8);
const TEAM: TeamId = TeamId::new(3);
const NOW: u64 = 1_000;

fn peers() -> Peers {
    Peers {
        control: ControlFace,
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
        op: hub_api::DMA_ENROLL, requester: 0, line: 7,
        name: "disk".into(), class: "virtio-blk".into(), back: env::PieToken::NONE,
    }).unwrap();
    assert_eq!(ledger.advance_enrollment(0, &mut enrollment::Enrollment), Some(Ok(())));
    let binding = ledger.claim(core::ClaimRequest {
        from: TASK, id: 1, requester: 0,
        sensor: env::PieToken::mint(TASK.get()), recovery: TaskId::new(3),
    }).unwrap();
    ledger.close(binding.serial, hub_api::Close::Normal).unwrap();
    assert_eq!(ledger.open_bindings_for(TASK), 1);
    let mut executions = Executions::try_new().unwrap();
    let peers = peers();
    backend::reset();

    assert!(executions.closed(&mut ledger, TASK));
    executions.advance(&mut ledger, &peers, NOW);
    assert_eq!(backend::began(Kind::Release), 1);
    assert!(executions.has_return(TASK));
    assert!(ledger.admission(TASK).is_some());
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

    backend::script_release(Ok(Some(ok_said(TASK, State::Dead))));
    executions.advance(&mut ledger, &peers, NOW);
    executions.advance(&mut ledger, &peers, NOW);
    executions.advance(&mut ledger, &peers, NOW);
    assert!(executions.has_return(TASK2));
    assert!(ledger.admission(TASK2).is_some());
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
