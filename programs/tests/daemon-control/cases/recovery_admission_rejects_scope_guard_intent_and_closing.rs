use crate::support::{active, admit, control, open, recovery, reset, target};
use crate::system::control::lifecycle::{Action, Fail};
use crate::system::control::unit::table::{Slot, State};

#[test]
fn recovery_admission_rejects_scope_guard_intent_and_closing() {
    reset();
    let mut out = control(true, true, State::Dead, Slot::None);
    out.targets = target(&["other"]);
    assert_eq!(
        admit(out, active(Action::Recover, recovery(), "worker"), open()),
        Err(Fail::Denied)
    );

    let mut guarded = control(true, true, State::Dead, Slot::None);
    guarded.guarded = &["worker"];
    assert_eq!(
        admit(guarded, active(Action::Recover, recovery(), "worker"), open()),
        Err(Fail::Denied)
    );

    let mut stopped = control(true, true, State::Dead, Slot::None);
    stopped.table.set_run_requested("worker", false).unwrap();
    assert_eq!(
        admit(stopped, active(Action::Recover, recovery(), "worker"), open()),
        Err(Fail::Denied)
    );

    assert_eq!(
        admit(
            control(true, true, State::Dead, Slot::None),
            active(Action::Recover, recovery(), "worker"),
            crate::system::app::policy::Settle { closing: true },
        ),
        Err(Fail::Denied)
    );
}
