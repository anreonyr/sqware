use crate::support::{active, admit, control, open, recovery, reset};
use crate::system::control::lifecycle::Action;
use crate::system::control::unit::table::{Slot, State};
use schedule::Progress;

#[test]
fn recovery_admission_accepts_only_a_named_supervisor_with_a_retired_target() {
    reset();
    let active = active(Action::Recover, recovery(), "worker");
    assert_eq!(
        admit(control(true, true, State::Dead, Slot::None), active, open()),
        Ok(Progress::Done)
    );
}
