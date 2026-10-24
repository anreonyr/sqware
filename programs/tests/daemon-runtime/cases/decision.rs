use crate::decision::{Decision, decide};
use system_api::control::frame::OK;
use system_api::control::observe::Reply;
use system_api::control::State;

fn reply(present: bool, state: State, run_requested: bool, end: bool) -> Reply {
    Reply {
        status: OK,
        index: 3,
        present,
        end,
        run_requested,
        state: state.code(),
        name: "rtc".into(),
    }
}

#[test]
fn only_present_dead_run_requested_recovers() {
    assert_eq!(
        decide(&reply(true, State::Dead, true, false)),
        Decision::Recover
    );
    assert_eq!(
        decide(&reply(true, State::Dead, false, false)),
        Decision::Advance
    );
    assert_eq!(
        decide(&reply(true, State::Starting, true, false)),
        Decision::Advance
    );
    assert_eq!(
        decide(&reply(true, State::Ready, true, false)),
        Decision::Advance
    );
}

#[test]
fn absent_and_unknown_states_advance_without_dead_interpretation() {
    assert_eq!(
        decide(&reply(false, State::Dead, true, false)),
        Decision::Advance
    );
    let mut unknown = reply(true, State::Dead, true, false);
    unknown.state = u8::MAX;
    assert_eq!(decide(&unknown), Decision::Advance);
}

#[test]
fn explicit_end_resets_the_enumeration() {
    assert_eq!(decide(&reply(false, State::NeverStarted, false, true)), Decision::End);
}
