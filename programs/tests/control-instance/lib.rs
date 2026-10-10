#![allow(dead_code)]
extern crate alloc;
extern crate env as abi;
extern crate self as env;
extern crate self as programs;

pub use abi::{ExitCause, JoinReply, Reason, TaskExit, TaskId, TeamId, UnitFail, UnitTarget, Wait};
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
    embark_fail: bool,
    embark_busy: bool,
    debark_fail: Option<UnitFail>,
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
pub mod debug { pub fn put(_: &str) {} }
pub mod unit {
    pub fn join(_: crate::UnitTarget, _: crate::Wait, receive: bool) -> Result<crate::JoinReply, crate::Error> {
        assert!(receive);
        crate::EFFECTS.with(|effects| Ok(effects.borrow_mut().exits.pop_front()
            .map_or(crate::JoinReply::Pending, crate::JoinReply::Reaped)))
    }
    pub fn join_task(_: crate::TaskId, _: crate::Wait) -> Result<bool, crate::Error> { Ok(true) }
    pub fn oust(_: crate::TeamId) -> Result<(), crate::Error> {
        crate::EFFECTS.with(|effects| { let mut effects = effects.borrow_mut(); effects.ousted += 1; effects.exits.clear(); });
        Ok(())
    }

    pub fn embark_task(task: crate::TaskId) -> Result<(), crate::Error> { embark(task) }
    pub fn embark_team(_: crate::TeamId) -> Result<(), crate::Error> { embark(crate::TaskId::new(0)) }
    pub fn slay_team(team: crate::TeamId) -> Result<(), crate::Error> {
        crate::EFFECTS.with(|effects| {
            let mut effects = effects.borrow_mut();
            effects.doomed += 1;
            effects.slain_teams.push(team);
        });
        Ok(())
    }
    pub fn debark_team(_: crate::TeamId) -> Result<(), crate::Error> { debark(crate::TaskId::new(0)) }

    use crate::{EFFECTS, Error, TaskId, UnitFail};
    pub fn embark(_: TaskId) -> Result<(), Error> {
        EFFECTS.with(|effects| {
            let mut effects = effects.borrow_mut();
            effects.embarked += 1;
            if effects.embark_busy { return Err(Error { source: UnitFail::Busy }); }
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
        pub enum Fault { Room }
        impl From<schedule::DispatchError> for Fault {
            fn from(_: schedule::DispatchError) -> Self { Self::Room }
        }
    }

    pub mod control {
        pub mod unit {
            pub mod table {
                pub use system_api::control::State;
            }
            pub struct Control {
                pub instances: alloc::vec::Vec<super::instance::state::Instance>,
            }
        }
        pub mod instance {
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
}

#[cfg(test)]
mod tests {
    use super::*;
    use system::control::{
        instance::Command, instance::state::Instance, unit::Control, unit::table::State,
    };
    use system_api::control::Fail;

    fn fixture(state: State) -> Control {
        EFFECTS.with(|effects| *effects.borrow_mut() = Effects::default());
        Control {
            instances: vec![Instance {
                owner: TaskId::new(1),
                task: TaskId::new(2),
                team: Some(TeamId::new(3)),
                state,
                started: matches!(state, State::Ready),
                reason: None,
                claimed: false,
                claim_until: 10,
                hook: Default::default(),
            }],
        }
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
                    (effects.embarked, effects.debarked, effects.doomed),
                    (0, 0, 0)
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
            Ok(Some(State::Ready))
        );
        assert!(control.instances[0].claimed);
        EFFECTS.with(|effects| effects.borrow_mut().now = 20);
        assert_eq!(
            call(&mut control, Command::Embark(TaskId::new(2))),
            Ok(Some(State::Ready))
        );
        EFFECTS.with(|effects| assert_eq!(effects.borrow().embarked, 1));
    }
    #[test]
    fn failed_embark_stops_without_claiming() {
        let mut control = fixture(State::Debarked);
        EFFECTS.with(|effects| effects.borrow_mut().embark_fail = true);
        assert_eq!(
            call(&mut control, Command::Embark(TaskId::new(2))),
            Err(Fail::NotReady)
        );
        assert_eq!(control.instances[0].state, State::Stopping);
        assert!(!control.instances[0].claimed);
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
        assert!(control.instances[0].team.is_some());
        control.instances[0].team = None;
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
    fn zero_main_exit_cannot_hide_an_auxiliary_failure() {
        let main = TaskExit { task: TaskId::new(2), cause: ExitCause::Reap, reason: 0 };
        let auxiliary = TaskExit { task: TaskId::new(4), cause: ExitCause::Fault, reason: 17 };
        for exits in [[main, auxiliary], [auxiliary, main]] {
            let mut control = fixture(State::Ready);
            for exit in exits { control.instances[0].reap(exit); }
            assert_eq!(control.instances[0].reason, Some(17));
            assert_eq!(control.instances[0].state, State::Stopping);
            control.instances[0].reap(TaskExit { reason: 23, ..main });
            assert_eq!(control.instances[0].reason, Some(23));
        }
        let mut control = fixture(State::Ready);
        control.instances[0].reap(TaskExit { cause: ExitCause::Slay, ..auxiliary });
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
                effects.exits.push_back(TaskExit { task: TaskId::new(id), cause: ExitCause::Reap, reason: 0 });
            }
            effects.exits.back_mut().unwrap().cause = ExitCause::Fault;
            effects.exits.back_mut().unwrap().reason = 17;
        });
        let mut resources = schedule::Resources::new();
        resources.insert(control).unwrap();
        resources.insert(hook::Active::default()).unwrap();
        resources.write::<hook::Active>().unwrap().task = Some(TaskId::new(2));
        for _ in 0..2 {
            assert_eq!(hook::reclaim(resources.write().unwrap(), resources.read().unwrap()), Ok(schedule::Progress::Pending));
            EFFECTS.with(|effects| assert_eq!(effects.borrow().ousted, 0));
        }
        assert_eq!(hook::reclaim(resources.write().unwrap(), resources.read().unwrap()), Ok(schedule::Progress::Done));
        assert_eq!(resources.read::<Control>().unwrap().instances[0].reason, Some(17));
        EFFECTS.with(|effects| assert_eq!(effects.borrow().ousted, 1));
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
}

#[test]
fn resuming_busy_team_keeps_transition_retriable() {
    use system::control::{instance::Command, instance::state::Instance, unit::Control, unit::table::State};
    EFFECTS.with(|e| *e.borrow_mut() = Effects { embark_busy: true, ..Effects::default() });
    let mut control = Control { instances: vec![Instance { owner: TaskId::new(1), task: TaskId::new(2), team: Some(TeamId::new(3)), state: State::Debarked, claimed: true, started: true, reason: None, claim_until: 0, hook: Default::default() }] };
    assert_eq!(control.command_instance(TaskId::new(1), Command::Embark(TaskId::new(2))), Ok(None));
    assert_eq!(control.instances[0].state, State::Debarked);
    EFFECTS.with(|e| e.borrow_mut().embark_busy = false);
    assert_eq!(control.command_instance(TaskId::new(1), Command::Embark(TaskId::new(2))), Ok(Some(State::Ready)));
}

#[test]
fn full_instance_table_rejects_before_construction() {
    use system::control::{instance::state::{Instance, INSTANCE_CAP}, unit::Control, unit::table::State};
    use system_api::{control::Fail, loader::Built};
    EFFECTS.with(|e| *e.borrow_mut() = Effects::default());
    let mut control = Control { instances: (0..INSTANCE_CAP).map(|n| Instance {
        owner: TaskId::new(1), task: TaskId::new(n + 2), team: Some(TeamId::new(n + 3)),
        state: State::Starting, claimed: false, started: false, reason: None,
        claim_until: 0, hook: Default::default(),
    }).collect() };
    let invoked = std::cell::Cell::new(false);
    let result = control.create_instance(TaskId::new(1), || {
        invoked.set(true); Ok(Built { task: TaskId::new(1000), team: TeamId::new(1001) })
    });
    assert!(matches!(result, Err(Fail::Full)));
    assert!(!invoked.get(), "capacity rejection must not create a kernel team");
    assert_eq!(control.instances.len(), INSTANCE_CAP);
    EFFECTS.with(|e| { let e = e.borrow(); assert_eq!((e.doomed, e.ousted), (0, 0)); });
}

#[test]
fn failed_construction_does_not_publish_an_instance() {
    use system::control::unit::Control;
    use system_api::control::Fail;
    EFFECTS.with(|e| *e.borrow_mut() = Effects::default());
    let mut control = Control { instances: Vec::new() };
    assert!(matches!(control.create_instance(TaskId::new(1), || Err(Fail::BadImage)), Err(Fail::BadImage)));
    assert!(control.instances.is_empty());
    EFFECTS.with(|e| { let e = e.borrow(); assert_eq!((e.doomed, e.ousted), (0, 0)); });
}

#[test]
fn successful_construction_is_registered_before_return() {
    use system::control::{unit::Control, unit::table::State};
    use system_api::loader::Built;
    EFFECTS.with(|e| *e.borrow_mut() = Effects { now: 42, ..Effects::default() });
    let mut control = Control { instances: Vec::new() };
    let built = control.create_instance(TaskId::new(7), || Ok(Built {
        task: TaskId::new(8), team: TeamId::new(9),
    })).unwrap();
    assert_eq!(built.task, TaskId::new(8));
    assert_eq!(control.instances.len(), 1);
    let instance = &control.instances[0];
    assert_eq!((instance.owner, instance.task, instance.team), (TaskId::new(7), built.task, Some(built.team)));
    assert_eq!(instance.state, State::Starting);
    assert!(!instance.claimed && !instance.started);
    assert_eq!(instance.claim_until, 42 + system_api::loader::CLAIM_MS as u64 * 1_000_000);
}
