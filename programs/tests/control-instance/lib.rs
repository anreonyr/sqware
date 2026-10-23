#![allow(dead_code)]
extern crate alloc;
extern crate env as abi;
extern crate self as env;
extern crate self as programs;

pub use abi::{ExitCause, JoinReply, Reason, TaskExit, TaskId, TeamId, UnitFail, UnitTarget, Wait};
pub use system::control::instance::receipt::Receipt;
use std::cell::RefCell;

#[derive(Default)]
struct Effects {
    now: u64,
    embarked: usize,
    debarked: usize,
    doomed: usize,
    slain_teams: Vec<TeamId>,
    exits: std::collections::VecDeque<TaskExit>,
    ousted: usize,
    oust_fail: Option<UnitFail>,
    oust_attempts: Vec<TeamId>,
    embark_fail: bool,
    embark_busy: bool,
    debark_fail: Option<UnitFail>,
    alive_tasks: Vec<TaskId>,
}
thread_local! {
    static EFFECTS: RefCell<Effects> = RefCell::new(Effects::default());
}
pub mod chrono {
    pub fn clock() -> u64 {
        crate::EFFECTS.with(|effects| effects.borrow().now)
    }
}
pub struct Error {
    pub source: UnitFail,
}
pub mod debug {
    pub fn put(_: &str) {}
}
pub mod unit {
    pub fn self_id() -> crate::TaskId {
        crate::TaskId::new(1)
    }

    pub fn join(
        _: crate::UnitTarget,
        _: crate::Wait,
        receive: bool,
    ) -> Result<crate::JoinReply, crate::Error> {
        Ok(crate::EFFECTS.with(|effects| {
            let mut effects = effects.borrow_mut();
            if !receive {
                return if effects.alive_tasks.is_empty() {
                    crate::JoinReply::Complete
                } else {
                    crate::JoinReply::Pending
                };
            }
            effects
                .exits
                .pop_front()
                .map_or(crate::JoinReply::Pending, crate::JoinReply::Reaped)
        }))
    }
    pub fn join_task(task: crate::TaskId, _: crate::Wait) -> Result<bool, crate::Error> {
        Ok(crate::EFFECTS.with(|effects| !effects.borrow().alive_tasks.contains(&task)))
    }
    pub fn oust(team: crate::TeamId) -> Result<(), crate::Error> {
        crate::EFFECTS.with(|effects| {
            let mut effects = effects.borrow_mut();
            effects.oust_attempts.push(team);
            if let Some(source) = effects.oust_fail { return Err(crate::Error { source }) }
            effects.ousted += 1;
            effects.exits.clear();
            Ok(())
        })
    }

    pub fn embark_task(task: crate::TaskId) -> Result<(), crate::Error> {
        embark(task)
    }
    pub fn embark_team(_: crate::TeamId) -> Result<(), crate::Error> {
        embark(crate::TaskId::new(0))
    }
    pub fn slay_team(team: crate::TeamId) -> Result<(), crate::Error> {
        crate::EFFECTS.with(|effects| {
            let mut effects = effects.borrow_mut();
            effects.doomed += 1;
            effects.slain_teams.push(team);
        });
        Ok(())
    }
    pub fn debark_team(_: crate::TeamId) -> Result<(), crate::Error> {
        debark(crate::TaskId::new(0))
    }

    use crate::{EFFECTS, Error, TaskId, UnitFail};
    pub fn embark(_: TaskId) -> Result<(), Error> {
        EFFECTS.with(|effects| {
            let mut effects = effects.borrow_mut();
            effects.embarked += 1;
            if effects.embark_busy {
                return Err(Error {
                    source: UnitFail::Busy,
                });
            }
            if effects.embark_fail {
                Err(Error {
                    source: UnitFail::Denied,
                })
            } else {
                Ok(())
            }
        })
    }
    pub fn debark(_: TaskId) -> Result<(), Error> {
        EFFECTS.with(|effects| {
            let mut effects = effects.borrow_mut();
            effects.debarked += 1;
            match effects.debark_fail {
                Some(source) => Err(Error { source }),
                None => Ok(()),
            }
        })
    }
}
pub mod room {
    pub fn doom(_: crate::TaskId) -> Result<(), ()> {
        crate::EFFECTS.with(|effects| effects.borrow_mut().doomed += 1);
        Ok(())
    }
}
#[path = "../../src/system/control/instance/create.rs"]
pub mod instance_create;
pub use system::control::instance::state;
#[path = "../../src/system/control/instance/hook.rs"]
pub mod instance_hook;

pub mod system {
    pub mod app {
        #[derive(Debug)]
        pub enum Fault {
            Room,
        }
        impl From<schedule::DispatchError> for Fault {
            fn from(_: schedule::DispatchError) -> Self {
                Self::Room
            }
        }
    }

    pub mod control {
        pub mod unit {
            pub mod verdict {
                pub use system_api::control::Fail;
            }
            pub mod table {
                pub use system_api::control::State;
            }
            pub struct Control {
                pub reclaimed_at: u64,
                pub instances: alloc::vec::Vec<super::instance::state::Instance>,
            }
        }
        pub mod instance {
            pub mod receipt {
                include!(concat!(env!("CARGO_MANIFEST_DIR"), "/../../src/system/control/instance/receipt.rs"));
            }
            mod release {
                include!(concat!(env!("CARGO_MANIFEST_DIR"), "/../../src/system/control/instance/release.rs"));
            }
            use crate::unit;
            pub mod activation {
                include!(concat!(
                    env!("CARGO_MANIFEST_DIR"),
                    "/../../src/system/control/instance/activation.rs"
                ));
            }
            pub use crate::instance_create as create;
            pub mod state {
                include!(concat!(
                    env!("CARGO_MANIFEST_DIR"),
                    "/../../src/system/control/instance/state.rs"
                ));
            }
            pub use crate::instance_hook as hook;
            #[cfg(test)]
            pub(crate) use command::Command;
            mod command {
                include!(concat!(
                    env!("CARGO_MANIFEST_DIR"),
                    "/../../src/system/control/instance/command.rs"
                ));
            }
        }
    }
    pub mod assembly {
        #[derive(Clone, Copy, PartialEq, Eq)]
        pub struct JobId(core::num::NonZeroU64);
        impl JobId {
            pub fn new(raw: u64) -> Option<Self> {
                core::num::NonZeroU64::new(raw).map(Self)
            }
            pub fn get(self) -> u64 {
                self.0.get()
            }
        }
        #[derive(Clone, Copy, PartialEq, Eq)]
        pub struct InstanceRef {
            pub job: JobId,
            pub task: crate::TaskId,
        }
        pub struct HookSelection {
            pub reference: InstanceRef,
            pub job: JobId,
            pub next: usize,
        }
        pub enum JobOutcome {
            Prepare(InstanceRef),
            Retire(InstanceRef),
        }
        pub enum JobFailure {
            Installation(&'static str),
            Construction(system_api::control::Fail),
            Control(system_api::control::Fail),
            Command(system_api::control::Fail),
        }
        pub struct JobCompletion {
            pub id: JobId,
            pub result: JobResult,
        }
        pub struct JobResult {
            pub outcome: Result<JobOutcome, JobFailure>,
        }
        pub fn reference(task: crate::TaskId) -> InstanceRef {
            InstanceRef {
                job: JobId::new(task.get() as u64).unwrap(),
                task,
            }
        }
        pub struct Jobs {
            pub active_instance: Option<InstanceRef>,
            active_job: Option<JobId>,
            phases: alloc::vec::Vec<(
                JobId,
                InstanceRef,
                crate::system::control::instance::hook::Key,
                schedule::Cursor,
            )>,
            completed: alloc::vec::Vec<JobCompletion>,
            next_job: u64,
            next: usize,
        }
        impl Default for Jobs {
            fn default() -> Self {
                Self {
                    active_instance: None,
                    active_job: None,
                    phases: alloc::vec::Vec::new(),
                    completed: alloc::vec::Vec::new(),
                    next_job: 1,
                    next: 0,
                }
            }
        }
        impl Jobs {
            pub fn phase(
                &mut self,
                reference: InstanceRef,
                key: crate::system::control::instance::hook::Key,
            ) -> Result<JobId, ()> {
                if let Some((id, ..)) = self
                    .phases
                    .iter()
                    .find(|(_, found, action, _)| *found == reference && *action == key)
                {
                    return Ok(*id);
                }
                let id = JobId::new(self.next_job).unwrap();
                self.next_job += 1;
                self.phases
                    .push((id, reference, key, schedule::Cursor::default()));
                Ok(id)
            }
            pub fn phase_cursor(&mut self, id: JobId) -> Option<&mut schedule::Cursor> {
                self.phases
                    .iter_mut()
                    .find(|item| item.0 == id)
                    .map(|item| &mut item.3)
            }
            pub fn take_phase(&mut self, id: JobId) -> Option<InstanceRef> {
                let at = self.phases.iter().position(|item| item.0 == id)?;
                Some(self.phases.remove(at).1)
            }
            pub fn complete_phase(
                &mut self,
                id: JobId,
                result: Result<JobOutcome, JobFailure>,
            ) -> bool {
                let Some(at) = self.phases.iter().position(|phase| phase.0 == id) else {
                    return false;
                };
                self.phases.remove(at);
                self.completed.push(JobCompletion {
                    id,
                    result: JobResult { outcome: result },
                });
                true
            }
            pub fn take(&mut self, id: JobId) -> Option<JobCompletion> {
                let at = self.completed.iter().position(|item| item.id == id)?;
                Some(self.completed.remove(at))
            }
            pub fn complete_create_prepare(&mut self, _: JobId, _: InstanceRef) -> bool {
                true
            }
            pub fn instance_ref(&self, task: crate::TaskId) -> Option<InstanceRef> {
                Some(reference(task))
            }
            pub fn active_hook_reference(&self) -> Option<InstanceRef> {
                self.active_instance
            }
            pub fn active_hook_task(&self) -> Option<crate::TaskId> {
                self.active_instance.map(|instance| instance.task)
            }
            pub fn select_hook(&mut self, selection: HookSelection) {
                self.active_instance = Some(selection.reference);
                self.active_job = Some(selection.job);
                self.next = selection.next;
            }
            pub fn active_hook_job(&self) -> Option<JobId> {
                self.active_job
            }
            pub fn advance_hook(&mut self, next: usize) {
                self.next = next;
            }
            pub fn next_hook(&self) -> usize {
                self.next
            }
            pub fn clear_hook(&mut self) {
                self.active_instance = None;
                self.active_job = None;
            }
            pub fn reset_hook_scan(&mut self) {
                self.active_instance = None;
                self.active_job = None;
                self.next = 0;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use system::control::{
        instance::Command, instance::state::Instance, unit::Control, unit::table::State,
    };
    use system_api::control::Fail;

    #[test]
    fn activation_wait_is_bounded_and_preserves_failure() {
        use system::control::instance::activation::bounded_ready;
        assert_eq!(bounded_ready(Ok(false), 99, 100), Ok(false));
        assert_eq!(bounded_ready(Ok(false), 100, 100), Err(Fail::NotReady));
        assert_eq!(bounded_ready(Ok(true), 100, 100), Ok(true));
        assert_eq!(
            bounded_ready(Err(Fail::Unknown), 1, 100),
            Err(Fail::Unknown)
        );
    }

    fn fixture(state: State) -> Control {
        EFFECTS.with(|effects| *effects.borrow_mut() = Effects::default());
        Control {
            reclaimed_at: 0,
            instances: vec![Instance {
                reference: crate::system::assembly::reference(TaskId::new(2)),
                owner: TaskId::new(1),
                task: TaskId::new(2),
                receipt: crate::Receipt::Owner(TeamId::new(3)),
                state,
                started: matches!(state, State::Ready),
                reason: None,
                claimed: false,
                claim_until: 10,
            }],
        }
    }
    #[test]
    fn pending_activation_claim_is_idempotent_without_extending_deadline() {
        let mut instance = Instance {
            reference: crate::system::assembly::reference(TaskId::new(2)),
            owner: TaskId::new(1),
            task: TaskId::new(2),
            receipt: crate::Receipt::Owner(TeamId::new(3)),
            state: State::Starting,
            started: true,
            reason: None,
            claimed: true,
            claim_until: 77,
        };
        assert_eq!(instance.claim(99), Some(TeamId::new(3)));
        assert_eq!(instance.claim_until, 77);

        instance.claimed = false;
        instance.started = false;
        assert_eq!(instance.claim(1), None);
    }
    fn call(control: &mut Control, command: Command) -> Result<Option<State>, Fail> {
        control.command_instance(TaskId::new(1), command)
    }
    #[test]
    fn foreign_owner_cannot_observe_or_change_an_instance() {
        for command in [
            Command::State(TaskId::new(2)),
            Command::Embark(TaskId::new(2)),
            Command::Debark(TaskId::new(2)),
            Command::Ruin(TaskId::new(2)),
            Command::Release(TaskId::new(2)),
        ] {
            let mut control = fixture(State::Debarked);
            assert_eq!(
                control.command_instance(TaskId::new(9), command),
                Err(Fail::Denied)
            );
            assert_eq!(control.instances[0].state, State::Debarked);
            EFFECTS.with(|effects| {
                let effects = effects.borrow();
                assert_eq!(
                    (effects.embarked, effects.debarked, effects.doomed, effects.ousted),
                    (0, 0, 0, 0)
                );
            });
        }
    }
    #[test]
    fn expired_claim_stops_without_embarking() {
        let mut control = fixture(State::Debarked);
        EFFECTS.with(|effects| effects.borrow_mut().now = 10);
        assert_eq!(
            call(&mut control, Command::Embark(TaskId::new(2))),
            Err(Fail::NotReady)
        );
        assert_eq!(control.instances[0].state, State::Stopping);
        assert!(!control.instances[0].claimed);
        EFFECTS.with(|effects| assert_eq!(effects.borrow().embarked, 0));
    }
    #[test]
    fn embark_claims_once_and_remains_idempotent_after_deadline() {
        let mut control = fixture(State::Debarked);
        assert_eq!(
            call(&mut control, Command::Embark(TaskId::new(2))),
            Ok(Some(State::Starting))
        );
        assert!(control.instances[0].claimed);
        EFFECTS.with(|effects| effects.borrow_mut().now = 20);
        assert_eq!(
            call(&mut control, Command::Embark(TaskId::new(2))),
            Ok(Some(State::Starting))
        );
        EFFECTS.with(|effects| assert_eq!(effects.borrow().embarked, 1));
    }
    #[test]
    fn failed_embark_stops_after_preserving_claim() {
        let mut control = fixture(State::Debarked);
        EFFECTS.with(|effects| effects.borrow_mut().embark_fail = true);
        assert_eq!(
            call(&mut control, Command::Embark(TaskId::new(2))),
            Err(Fail::NotReady)
        );
        assert_eq!(control.instances[0].state, State::Stopping);
        assert!(control.instances[0].claimed);
    }
    #[test]
    fn busy_debark_is_pending_and_can_be_retried() {
        let mut control = fixture(State::Ready);
        EFFECTS.with(|effects| effects.borrow_mut().debark_fail = Some(UnitFail::Busy));
        assert_eq!(
            call(&mut control, Command::Debark(TaskId::new(2))),
            Ok(None)
        );
        assert_eq!(control.instances[0].state, State::Ready);
        EFFECTS.with(|effects| effects.borrow_mut().debark_fail = None);
        assert_eq!(
            call(&mut control, Command::Debark(TaskId::new(2))),
            Ok(Some(State::Debarked))
        );
    }
    #[test]
    fn ruin_waits_for_reclamation_before_completion() {
        let mut control = fixture(State::Ready);
        assert_eq!(call(&mut control, Command::Ruin(TaskId::new(2))), Ok(None));
        assert_eq!(control.instances[0].state, State::Stopping);
        assert!(control.instances[0].team().is_some());
        control.instances[0].state = State::Dead;
        assert_eq!(
            call(&mut control, Command::Ruin(TaskId::new(2))),
            Ok(Some(State::Dead))
        );
        EFFECTS.with(|effects| {
            let effects = effects.borrow();
            assert_eq!(effects.doomed, 1);
            assert_eq!(effects.slain_teams, vec![TeamId::new(3)]);
        });
    }

    #[test]
    fn release_retries_original_receipt_and_never_confirms_a_failed_oust() {
        let mut control = fixture(State::Dead);
        for error in [UnitFail::Busy, UnitFail::Denied, UnitFail::OoM] {
            EFFECTS.with(|effects| effects.borrow_mut().oust_fail = Some(error));
            let expected = if error == UnitFail::Busy { Ok(None) } else { Err(Fail::NotReady) };
            assert_eq!(call(&mut control, Command::Release(TaskId::new(2))), expected);
            assert_eq!(control.instances[0].receipt, Receipt::Returned(TeamId::new(3)));
            assert_eq!(control.instances[0].team(), None);
            assert_eq!(control.reclaimed_at, 0);
            EFFECTS.with(|effects| assert_eq!(effects.borrow().ousted, 0));
        }
        EFFECTS.with(|effects| {
            let mut effects = effects.borrow_mut();
            effects.oust_fail = None;
            effects.now = 84;
        });
        assert_eq!(call(&mut control, Command::Release(TaskId::new(2))), Ok(Some(State::Dead)));
        assert_eq!(control.instances[0].receipt, Receipt::Consumed);
        assert_eq!(control.reclaimed_at, 84);
        EFFECTS.with(|effects| effects.borrow_mut().now = 168);
        assert_eq!(call(&mut control, Command::Release(TaskId::new(2))), Ok(Some(State::Dead)));
        assert_eq!(control.reclaimed_at, 84);
        EFFECTS.with(|effects| {
            let effects = effects.borrow();
            assert_eq!(effects.ousted, 1);
            assert_eq!(effects.oust_attempts, vec![TeamId::new(3); 4]);
        });
        assert_eq!(call(&mut control, Command::Release(TaskId::new(99))), Err(Fail::Unknown));
    }

    #[test]
    fn returned_receipt_waits_for_real_whole_team_completion_and_hook_reclamation() {
        use system::control::instance::hook;
        let resources = hook_resources(State::Ready);
        assert_eq!(call(&mut resources.write::<Control>().unwrap(), Command::Release(TaskId::new(2))), Ok(None));
        EFFECTS.with(|effects| effects.borrow_mut().alive_tasks.push(TaskId::new(4)));
        let mut child = schedule::Schedule::sequence();
        child.system("reclaim", hook::reclaim).unwrap();
        let mut plan = hook::plan(vec![(hook::Key::Retire, child.build().unwrap())]).unwrap();
        plan.prepare(&resources);
        plan.advance(&mut schedule::Cursor::default(), &resources).unwrap();
        assert_eq!(resources.read::<Control>().unwrap().instances[0].receipt, Receipt::Returned(TeamId::new(3)));
        EFFECTS.with(|effects| assert!(effects.borrow().oust_attempts.is_empty()));
        EFFECTS.with(|effects| effects.borrow_mut().alive_tasks.clear());
        plan.advance(&mut schedule::Cursor::default(), &resources).unwrap();
        let control = resources.read::<Control>().unwrap();
        assert_eq!(control.instances[0].state, State::Dead);
        assert_eq!(control.instances[0].receipt, Receipt::Consumed);
        assert_eq!(control.reclaimed_at, 42);
        EFFECTS.with(|effects| assert_eq!(effects.borrow().oust_attempts, [TeamId::new(3)]));
    }

    #[test]
    fn returned_receipt_is_retried_without_an_owner_reply_or_another_command() {
        let mut control = fixture(State::Dead);
        EFFECTS.with(|effects| effects.borrow_mut().oust_fail = Some(UnitFail::Busy));
        assert_eq!(call(&mut control, Command::Release(TaskId::new(2))), Ok(None));
        control.reclaim_returned();
        assert_eq!(control.instances[0].receipt, Receipt::Returned(TeamId::new(3)));
        EFFECTS.with(|effects| {
            let mut effects = effects.borrow_mut();
            effects.oust_fail = None;
            effects.now = 91;
        });
        control.reclaim_returned();
        assert_eq!(control.instances[0].receipt, Receipt::Consumed);
        assert_eq!(control.reclaimed_at, 91);
        EFFECTS.with(|effects| effects.borrow_mut().now = 182);
        control.reclaim_returned();
        assert_eq!(control.reclaimed_at, 91);
        EFFECTS.with(|effects| assert_eq!(effects.borrow().ousted, 1));
    }

    #[test]
    fn legacy_retire_preserves_owner_receipt_for_late_release() {
        let resources = hook_resources(State::Stopping);
        run_hook(&resources, system::control::instance::hook::Key::Retire, Ok(schedule::Progress::Done));
        {
            let control = resources.read::<Control>().unwrap();
            assert_eq!(control.instances[0].state, State::Dead);
            assert_eq!(control.instances[0].team(), None);
            assert_eq!(control.instances[0].receipt, Receipt::Owner(TeamId::new(3)));
        }
        EFFECTS.with(|effects| assert_eq!(effects.borrow().ousted, 0));
        assert_eq!(call(&mut resources.write::<Control>().unwrap(), Command::Release(TaskId::new(2))), Ok(Some(State::Dead)));
        assert_eq!(resources.read::<Control>().unwrap().instances[0].receipt, Receipt::Consumed);
        EFFECTS.with(|effects| assert_eq!(effects.borrow().ousted, 1));
    }

    #[test]
    fn retire_dispatch_can_finish_after_kernel_receipt_has_been_consumed() {
        let resources = hook_resources(State::Stopping);
        {
            let mut control = resources.write::<Control>().unwrap();
            control.instances[0].receipt = Receipt::Consumed;
            control.reclaimed_at = 21;
        }
        EFFECTS.with(|effects| effects.borrow_mut().alive_tasks.push(TaskId::new(99)));
        run_hook(&resources, system::control::instance::hook::Key::Retire, Ok(schedule::Progress::Done));
        let control = resources.read::<Control>().unwrap();
        assert_eq!(control.instances[0].state, State::Dead);
        assert_eq!(control.instances[0].receipt, Receipt::Consumed);
        assert_eq!(control.reclaimed_at, 21);
        EFFECTS.with(|effects| assert!(effects.borrow().oust_attempts.is_empty()));
    }

    #[test]
    fn unreturned_or_pending_receipts_cannot_be_evicted_to_build_another_instance() {
        use system::control::instance::create::Admission;
        use system_api::loader::Built;
        for receipt in [Receipt::Owner(TeamId::new(3)), Receipt::Returned(TeamId::new(3)), Receipt::Consumed] {
            let mut control = fixture(State::Dead);
            control.instances[0].receipt = receipt;
            for at in 1..state::INSTANCE_CAP {
                let mut item = Instance {
                    reference: crate::system::assembly::reference(TaskId::new(at + 2)),
                    owner: TaskId::new(1), task: TaskId::new(at + 2),
                    receipt: Receipt::Owner(TeamId::new(at + 3)),
                    state: State::Dead, claimed: false, started: false,
                    reason: None, claim_until: 0,
                };
                item.stop();
                assert_eq!(item.state, State::Dead);
                control.instances.push(item);
            }
            let invoked = std::cell::Cell::new(false);
            let result = control.create_instance(Admission {
                owner: TaskId::new(1), job: crate::system::assembly::JobId::new(999).unwrap(),
            }, || {
                invoked.set(true);
                Ok(Built { task: TaskId::new(999), team: TeamId::new(1000) })
            });
            assert_eq!(invoked.get(), receipt == Receipt::Consumed);
            if receipt == Receipt::Consumed { assert!(result.is_ok()); }
            else { assert!(matches!(result, Err(Fail::Full))); }
            assert_eq!(control.instances.len(), state::INSTANCE_CAP);
        }
    }

    #[test]
    fn consumed_receipt_is_retained_until_its_owner_can_receive_completion() {
        use system::control::instance::create::Admission;
        use system_api::loader::Built;
        let mut control = fixture(State::Dead);
        let owner = TaskId::new(99);
        control.instances[0].owner = owner;
        EFFECTS.with(|effects| effects.borrow_mut().alive_tasks.push(owner));
        assert_eq!(control.release_instance(owner, TaskId::new(2)), Ok(Some(State::Dead)));
        assert_eq!(control.instances[0].receipt, Receipt::Consumed);
        for at in 1..state::INSTANCE_CAP {
            control.instances.push(Instance {
                reference: crate::system::assembly::reference(TaskId::new(at + 100)),
                owner, task: TaskId::new(at + 100),
                receipt: Receipt::Owner(TeamId::new(at + 100)), state: State::Dead,
                claimed: false, started: false, reason: None, claim_until: 0,
            });
        }
        let invoked = std::cell::Cell::new(false);
        let result = control.create_instance(Admission {
            owner, job: crate::system::assembly::JobId::new(9999).unwrap(),
        }, || {
            invoked.set(true);
            Ok(Built { task: TaskId::new(9999), team: TeamId::new(9999) })
        });
        assert!(matches!(result, Err(Fail::Full)), "completion was evicted before delivery");
        assert!(!invoked.get());
        assert_eq!(control.release_instance(owner, TaskId::new(2)), Ok(Some(State::Dead)));
        EFFECTS.with(|effects| assert_eq!(effects.borrow().ousted, 1));
        let reference = control.instances[0].reference;
        assert!(!control.release_reported(TaskId::new(98), reference));
        assert!(!control.release_reported(owner,
            crate::system::assembly::reference(TaskId::new(98))));
        assert_eq!(control.instances[0].receipt, Receipt::Consumed);
        assert!(control.release_reported(owner, reference));
        assert_eq!(control.instances[0].receipt, Receipt::Reported);
        assert!(control.create_instance(Admission {
            owner, job: crate::system::assembly::JobId::new(9999).unwrap(),
        }, || Ok(Built { task: TaskId::new(9999), team: TeamId::new(9999) })).is_ok());
        assert!(control.instances.iter().all(|item| item.task != TaskId::new(2)));
        assert_eq!(control.instances.len(), state::INSTANCE_CAP);
    }
    #[test]
    fn zero_main_exit_cannot_hide_an_auxiliary_failure() {
        let main = TaskExit {
            task: TaskId::new(2),
            cause: ExitCause::Reap,
            reason: 0,
        };
        let auxiliary = TaskExit {
            task: TaskId::new(4),
            cause: ExitCause::Fault,
            reason: 17,
        };
        for exits in [[main, auxiliary], [auxiliary, main]] {
            let mut control = fixture(State::Ready);
            for exit in exits {
                control.instances[0].reap(exit);
            }
            assert_eq!(control.instances[0].reason, Some(17));
            assert_eq!(control.instances[0].state, State::Stopping);
            control.instances[0].reap(TaskExit { reason: 23, ..main });
            assert_eq!(control.instances[0].reason, Some(23));
        }
        let mut control = fixture(State::Ready);
        control.instances[0].reap(TaskExit {
            cause: ExitCause::Slay,
            ..auxiliary
        });
        assert_eq!(control.instances[0].state, State::Ready);
        assert_eq!(control.instances[0].reason, None);
    }
    #[test]
    fn reclamation_drains_late_auxiliary_failures_before_oust() {
        use system::control::instance::hook;
        let mut control = fixture(State::Stopping);
        control.instances[0].reason = Some(0);
        EFFECTS.with(|effects| {
            let mut effects = effects.borrow_mut();
            for id in 4..134 {
                effects.exits.push_back(TaskExit {
                    task: TaskId::new(id),
                    cause: ExitCause::Reap,
                    reason: 0,
                });
            }
            effects.exits.back_mut().unwrap().cause = ExitCause::Fault;
            effects.exits.back_mut().unwrap().reason = 17;
        });
        let mut resources = schedule::Resources::new();
        resources.insert(control).unwrap();
        resources.insert(system::assembly::Jobs::default()).unwrap();
        resources
            .write::<system::assembly::Jobs>()
            .unwrap()
            .active_instance = Some(system::assembly::reference(TaskId::new(2)));
        for _ in 0..2 {
            assert_eq!(
                hook::reclaim(resources.write().unwrap(), resources.read().unwrap()),
                Ok(schedule::Progress::Pending)
            );
            EFFECTS.with(|effects| assert_eq!(effects.borrow().ousted, 0));
        }
        assert_eq!(
            hook::reclaim(resources.write().unwrap(), resources.read().unwrap()),
            Ok(schedule::Progress::Done)
        );
        assert_eq!(
            resources.read::<Control>().unwrap().instances[0].reason,
            Some(17)
        );
        EFFECTS.with(|effects| assert_eq!(effects.borrow().ousted, 1));
    }

    #[test]
    fn unclaimed_delegated_instance_keeps_receipt_while_owner_is_alive() {
        use system::control::instance::hook;
        let mut control = fixture(State::Stopping);
        control.instances[0].claimed = false;
        control.instances[0].owner = TaskId::new(7);
        EFFECTS.with(|effects| effects.borrow_mut().alive_tasks.push(TaskId::new(7)));
        let mut resources = schedule::Resources::new();
        resources.insert(control).unwrap();
        let mut jobs = system::assembly::Jobs::default();
        jobs.active_instance = Some(system::assembly::reference(TaskId::new(2)));
        resources.insert(jobs).unwrap();
        assert_eq!(
            hook::reclaim(resources.write().unwrap(), resources.read().unwrap()),
            Ok(schedule::Progress::Done)
        );
        EFFECTS.with(|effects| assert_eq!(effects.borrow().ousted, 0));
        EFFECTS.with(|effects| effects.borrow_mut().alive_tasks.clear());
        assert_eq!(
            hook::reclaim(resources.write().unwrap(), resources.read().unwrap()),
            Ok(schedule::Progress::Done)
        );
        EFFECTS.with(|effects| assert_eq!(effects.borrow().ousted, 1));
    }

    #[test]
    fn retire_hook_waits_for_every_team_member_to_exit() {
        use system::control::instance::hook;
        fn set_alive(tasks: &[TaskId]) {
            EFFECTS.with(|effects| {
                let mut effects = effects.borrow_mut();
                effects.alive_tasks.clear();
                effects.alive_tasks.extend_from_slice(tasks);
            });
        }
        fn selects(resources: &schedule::Resources<'static>) -> bool {
            assert!(matches!(
                hook::budget(
                    resources.read().unwrap(),
                    resources.write().unwrap(),
                    resources.write().unwrap()
                ),
                Ok(schedule::Progress::Done)
            ));
            assert!(matches!(
                hook::select(
                    resources.read().unwrap(),
                    resources.write().unwrap(),
                    resources.write().unwrap()
                ),
                Ok(schedule::Progress::Done)
            ));
            resources
                .read::<system::assembly::Jobs>()
                .unwrap()
                .active_hook_reference()
                .is_some()
        }

        let mut control = fixture(State::Ready);
        control.instances[0].reap(TaskExit {
            task: TaskId::new(2),
            cause: ExitCause::Reap,
            reason: 0,
        });
        assert_eq!(control.instances[0].state, State::Stopping);
        // 主线程 2 已退，辅助成员 4 仍在：Team 尚未 Complete。
        set_alive(&[TaskId::new(4)]);
        let mut resources = schedule::Resources::new();
        resources.insert(control).unwrap();
        resources.insert(system::assembly::Jobs::default()).unwrap();
        resources
            .insert(schedule::Dispatch::<
                system::control::instance::hook::Key,
                &'static str,
            >::new())
            .unwrap();
        assert!(!selects(&resources));
        set_alive(&[]);
        assert!(selects(&resources));
        assert!(
            resources
                .read::<system::assembly::Jobs>()
                .unwrap()
                .active_hook_reference() ==
            Some(system::assembly::reference(TaskId::new(2)))
        );
    }

    #[test]
    fn missing_instance_and_invalid_transition_have_no_effects() {
        let mut control = fixture(State::Starting);
        assert_eq!(
            call(&mut control, Command::State(TaskId::new(9))),
            Err(Fail::Unknown)
        );
        assert_eq!(
            call(&mut control, Command::Embark(TaskId::new(2))),
            Err(Fail::NotReady)
        );
        assert_eq!(
            call(&mut control, Command::State(TaskId::new(2))),
            Ok(Some(State::Starting))
        );
        EFFECTS.with(|effects| {
            let effects = effects.borrow();
            assert_eq!(
                (effects.embarked, effects.debarked, effects.doomed),
                (0, 0, 0)
            );
        });
    }

    fn hook_resources(state: State) -> schedule::Resources<'static> {
        let control = fixture(state);
        EFFECTS.with(|effects| effects.borrow_mut().now = 42);
        let mut resources = schedule::Resources::new();
        resources.insert(control).unwrap();
        resources.insert(system::assembly::Jobs::default()).unwrap();
        resources.insert(
            schedule::Dispatch::<system::control::instance::hook::Key, &'static str>::new(),
        ).unwrap();
        resources
    }

    fn run_hook(
        resources: &schedule::Resources<'static>,
        key: system::control::instance::hook::Key,
        result: Result<schedule::Progress, &'static str>,
    ) {
        let mut child = schedule::Schedule::sequence();
        child.system("effect", move || result).unwrap();
        let mut plan = system::control::instance::hook::plan(vec![(key, child.build().unwrap())]).unwrap();
        plan.prepare(resources);
        plan.advance(&mut schedule::Cursor::default(), resources).unwrap();
    }

    #[test]
    fn only_successful_retire_hook_records_reclamation_time() {
        use system::control::instance::hook::Key;
        let resources = hook_resources(State::Stopping);
        run_hook(&resources, Key::Retire, Ok(schedule::Progress::Done));
        let control = resources.read::<Control>().unwrap();
        assert_eq!(control.reclaimed_at, 42);
        assert_eq!(control.instances[0].team(), None);
        assert_eq!(control.instances[0].state, State::Dead);
    }

    #[test]
    fn pending_or_failed_retire_hook_cannot_record_reclamation() {
        use system::control::instance::hook::Key;
        for result in [Ok(schedule::Progress::Pending), Err("cleanup failed")] {
            let resources = hook_resources(State::Stopping);
            run_hook(&resources, Key::Retire, result);
            let control = resources.read::<Control>().unwrap();
            assert_eq!(control.reclaimed_at, 0);
            assert_eq!(control.instances[0].team(), Some(TeamId::new(3)));
            assert_eq!(control.instances[0].state, State::Stopping);
        }
    }

    #[test]
    fn prepare_hook_is_not_reclamation_progress() {
        use system::control::instance::hook::Key;
        let resources = hook_resources(State::Starting);
        run_hook(&resources, Key::Prepare, Ok(schedule::Progress::Done));
        let control = resources.read::<Control>().unwrap();
        assert_eq!(control.reclaimed_at, 0);
        assert_eq!(control.instances[0].team(), Some(TeamId::new(3)));
        assert_eq!(control.instances[0].state, State::Debarked);
    }

    #[test]
    fn failed_prepare_cannot_be_claimed_or_embarked_before_retirement() {
        use system::control::instance::hook::Key;
        let resources = hook_resources(State::Starting);
        run_hook(&resources, Key::Prepare, Err("bootstrap push"));
        {
            let mut control = resources.write::<Control>().unwrap();
            assert_eq!(control.reclaimed_at, 0);
            let item = &mut control.instances[0];
            assert_eq!(item.state, State::Stopping);
            assert_eq!(item.team(), Some(TeamId::new(3)));
            assert!(!item.started);
            assert_eq!(item.claim(42), None);
        }
        EFFECTS.with(|effects| assert_eq!(effects.borrow().embarked, 0));
        run_hook(&resources, Key::Retire, Ok(schedule::Progress::Done));
        let control = resources.read::<Control>().unwrap();
        assert_eq!(control.instances[0].state, State::Dead);
        assert_eq!(control.instances[0].team(), None);
    }

    #[test]
    fn repeated_retire_completion_cannot_refresh_reclamation_time() {
        use system::control::instance::hook::Key;
        let resources = hook_resources(State::Stopping);
        run_hook(&resources, Key::Retire, Ok(schedule::Progress::Done));
        EFFECTS.with(|effects| effects.borrow_mut().now = 84);
        resources.write::<Control>().unwrap().instances[0].stop();
        assert_eq!(resources.read::<Control>().unwrap().instances[0].state, State::Dead);
        run_hook(&resources, Key::Retire, Ok(schedule::Progress::Done));
        let control = resources.read::<Control>().unwrap();
        assert_eq!(control.reclaimed_at, 42);
        assert_eq!(control.instances[0].team(), None);
    }
}

#[test]
fn resuming_busy_team_keeps_transition_retriable() {
    use system::control::{
        instance::Command, instance::state::Instance, unit::Control, unit::table::State,
    };
    EFFECTS.with(|e| {
        *e.borrow_mut() = Effects {
            embark_busy: true,
            ..Effects::default()
        }
    });
    let mut control = Control {
        reclaimed_at: 0,
        instances: vec![Instance {
            reference: crate::system::assembly::reference(TaskId::new(2)),
            owner: TaskId::new(1),
            task: TaskId::new(2),
            receipt: crate::Receipt::Owner(TeamId::new(3)),
            state: State::Debarked,
            claimed: true,
            started: true,
            reason: None,
            claim_until: 0,
        }],
    };
    assert_eq!(
        control.command_instance(TaskId::new(1), Command::Embark(TaskId::new(2))),
        Ok(None)
    );
    assert_eq!(control.instances[0].state, State::Debarked);
    EFFECTS.with(|e| e.borrow_mut().embark_busy = false);
    assert_eq!(
        control.command_instance(TaskId::new(1), Command::Embark(TaskId::new(2))),
        Ok(Some(State::Ready))
    );
}

#[test]
fn full_instance_table_rejects_before_construction() {
    use system::control::{
        instance::state::{INSTANCE_CAP, Instance},
        unit::Control,
        unit::table::State,
    };
    use system_api::{control::Fail, loader::Built};
    EFFECTS.with(|e| *e.borrow_mut() = Effects::default());
    let mut control = Control {
        reclaimed_at: 0,
        instances: (0..INSTANCE_CAP)
            .map(|n| Instance {
                reference: crate::system::assembly::reference(TaskId::new(n + 2)),
                owner: TaskId::new(1),
                task: TaskId::new(n + 2),
                receipt: crate::Receipt::Owner(TeamId::new(n + 3)),
                state: State::Starting,
                claimed: false,
                started: false,
                reason: None,
                claim_until: 0,
            })
            .collect(),
    };
    let invoked = std::cell::Cell::new(false);
    let result = control.create_instance(
        system::control::instance::create::Admission {
            owner: TaskId::new(1),
            job: crate::system::assembly::JobId::new(100).unwrap(),
        },
        || {
            invoked.set(true);
            Ok(Built {
                task: TaskId::new(1000),
                team: TeamId::new(1001),
            })
        },
    );
    assert!(matches!(result, Err(Fail::Full)));
    assert!(
        !invoked.get(),
        "capacity rejection must not create a kernel team"
    );
    assert_eq!(control.instances.len(), INSTANCE_CAP);
    EFFECTS.with(|e| {
        let e = e.borrow();
        assert_eq!((e.doomed, e.ousted), (0, 0));
    });
}

#[test]
fn failed_construction_does_not_publish_an_instance() {
    use system::control::unit::Control;
    use system_api::control::Fail;
    EFFECTS.with(|e| *e.borrow_mut() = Effects::default());
    let mut control = Control {
        reclaimed_at: 0,
        instances: Vec::new(),
    };
    assert!(matches!(
        control.create_instance(
            system::control::instance::create::Admission {
                owner: TaskId::new(1),
                job: crate::system::assembly::JobId::new(101).unwrap()
            },
            || Err(Fail::BadImage)
        ),
        Err(Fail::BadImage)
    ));
    assert!(control.instances.is_empty());
    EFFECTS.with(|e| {
        let e = e.borrow();
        assert_eq!((e.doomed, e.ousted), (0, 0));
    });
}

#[test]
fn successful_construction_is_registered_before_return() {
    use system::control::{unit::Control, unit::table::State};
    use system_api::loader::Built;
    EFFECTS.with(|e| {
        *e.borrow_mut() = Effects {
            now: 42,
            ..Effects::default()
        }
    });
    let mut control = Control {
        reclaimed_at: 0,
        instances: Vec::new(),
    };
    let built = control
        .create_instance(
            system::control::instance::create::Admission {
                owner: TaskId::new(7),
                job: crate::system::assembly::JobId::new(102).unwrap(),
            },
            || {
                Ok(Built {
                    task: TaskId::new(8),
                    team: TeamId::new(9),
                })
            },
        )
        .unwrap();
    assert_eq!(built.task, TaskId::new(8));
    assert_eq!(control.reclaimed_at, 0, "creation is not reclamation progress");
    assert_eq!(control.instances.len(), 1);
    let instance = &control.instances[0];
    assert_eq!(
        (instance.owner, instance.task, instance.team()),
        (TaskId::new(7), built.task, Some(built.team))
    );
    assert_eq!(instance.state, State::Starting);
    assert!(!instance.claimed && !instance.started);
    assert_eq!(
        instance.claim_until, 0,
        "the claim window starts after prepare"
    );
}

#[test]
fn one_owner_can_register_multiple_live_instances() {
    use system::control::{unit::Control, unit::table::State};
    use system_api::loader::Built;
    EFFECTS.with(|e| *e.borrow_mut() = Effects::default());
    let mut control = Control {
        reclaimed_at: 0,
        instances: Vec::new(),
    };
    for (task, team) in [(8, 9), (10, 11)] {
        control
            .create_instance(
                system::control::instance::create::Admission {
                    owner: TaskId::new(7),
                    job: crate::system::assembly::JobId::new(task as u64).unwrap(),
                },
                || {
                    Ok(Built {
                        task: TaskId::new(task),
                        team: TeamId::new(team),
                    })
                },
            )
            .unwrap();
    }
    assert_eq!(control.instances.len(), 2);
    assert!(
        control
            .instances
            .iter()
            .all(|item| item.owner == TaskId::new(7))
    );
    assert_eq!(control.instances[0].state, State::Starting);
    assert_eq!(control.instances[1].state, State::Starting);
    for item in &mut control.instances {
        item.state = State::Debarked;
        item.claim_until = 10;
    }
    assert_eq!(control.instances[0].claim(0), Some(TeamId::new(9)));
    assert!(control.instances[0].claimed);
    assert!(!control.instances[1].claimed);
    assert_eq!(control.instances[1].claim(0), Some(TeamId::new(11)));
    control.instances[0].stop();
    assert_eq!(control.instances[0].state, State::Stopping);
    assert_eq!(control.instances[1].state, State::Debarked);
    assert!(control.instances[1].claimed);
}

#[test]
fn repeated_claim_after_deadline_keeps_the_successful_claim() {
    use system::control::{instance::state::Instance, unit::table::State};
    let mut item = Instance {
        reference: crate::system::assembly::reference(TaskId::new(2)),
        owner: TaskId::new(1),
        task: TaskId::new(2),
        receipt: crate::Receipt::Owner(TeamId::new(3)),
        state: State::Debarked,
        claimed: false,
        started: false,
        reason: None,
        claim_until: 10,
    };
    assert_eq!(item.claim(9), Some(TeamId::new(3)));
    assert!(item.claimed);
    assert_eq!(item.claim(20), Some(TeamId::new(3)));
    assert_eq!(item.claim_until, 10);
}

#[test]
fn expired_unclaimed_instance_cannot_be_claimed() {
    use system::control::{instance::state::Instance, unit::table::State};
    let mut item = Instance {
        reference: crate::system::assembly::reference(TaskId::new(2)),
        owner: TaskId::new(1),
        task: TaskId::new(2),
        receipt: crate::Receipt::Owner(TeamId::new(3)),
        state: State::Debarked,
        claimed: false,
        started: false,
        reason: None,
        claim_until: 10,
    };
    assert_eq!(item.claim(10), None);
    assert!(!item.claimed);
}

#[test]
fn claimed_instance_does_not_inherit_a_child_job_failure() {
    use system::control::{instance::state::Instance, unit::table::State};
    let mut item = Instance {
        reference: crate::system::assembly::reference(TaskId::new(2)),
        owner: TaskId::new(1),
        task: TaskId::new(2),
        receipt: crate::Receipt::Owner(TeamId::new(3)),
        state: State::Ready,
        claimed: true,
        started: true,
        reason: None,
        claim_until: 0,
    };
    item.reap(TaskExit {
        task: TaskId::new(9),
        cause: ExitCause::Reap,
        reason: 17,
    });
    assert_eq!(item.state, State::Ready);
    assert_eq!(item.reason, None);
    item.reap(TaskExit {
        task: item.task,
        cause: ExitCause::Reap,
        reason: 0,
    });
    assert_eq!(item.state, State::Stopping);
    assert_eq!(item.reason, Some(0));
}
