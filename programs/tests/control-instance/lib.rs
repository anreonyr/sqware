#![allow(dead_code)]
extern crate alloc;
extern crate env as abi;
extern crate self as env;

pub use abi::{Reason, TaskId, TeamId, UnitFail};
use std::cell::RefCell;

#[derive(Default)]
struct Effects {
    now: u64,
    embarked: usize,
    debarked: usize,
    doomed: usize,
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
pub mod unit {
    pub fn embark_team(_: crate::TeamId) -> Result<(), crate::Error> { embark(crate::TaskId::new(0)) }
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
pub mod system {
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
            pub mod state {
                include!(concat!(
                    env!("CARGO_MANIFEST_DIR"),
                    "/../../src/system/control/instance/state.rs"
                ));
            }
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
        EFFECTS.with(|effects| assert_eq!(effects.borrow().doomed, 1));
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
