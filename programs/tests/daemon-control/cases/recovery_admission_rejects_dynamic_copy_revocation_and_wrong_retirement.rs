use crate::support::{active, admit, control, open, recovery, reset};
use crate::system::control::lifecycle::{Action, Fail};
use crate::system::control::unit::table::{Slot, State};

#[test]
fn recovery_admission_rejects_dynamic_copy_revocation_and_wrong_retirement() {
    reset();
    assert_eq!(
        admit(
            control(false, true, State::Dead, Slot::None),
            active(Action::Recover, recovery(), "worker"),
            open(),
        ),
        Err(Fail::Denied)
    );
    crate::DEAD.with(|dead| dead.borrow_mut().push(crate::TaskId::new(10)));
    assert_eq!(
        admit(
            control(true, true, State::Dead, Slot::None),
            active(Action::Recover, recovery(), "worker"),
            open(),
        ),
        Err(Fail::Denied)
    );
    reset();
    assert_eq!(
        admit(
            control(
                true,
                true,
                State::Starting,
                Slot::Live {
                    team: None,
                    task: crate::TaskId::new(20),
                },
            ),
            active(Action::Recover, recovery(), "worker"),
            open(),
        ),
        Err(Fail::Denied)
    );
}
