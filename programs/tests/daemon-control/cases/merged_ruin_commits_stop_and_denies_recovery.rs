use crate::fixtures::Waiters;
use crate::support::{
    active, admit, control, open, recovery, request, reset, sender, tracked, tracked_with_waiters,
};
use crate::system::control::lifecycle::{self, Action, Fail, Origin};
use crate::system::control::unit::table::{Slot, State};

fn merge(control: &mut crate::fixtures::Control, state: State) {
    let slot = Slot::Live {
        team: None,
        task: crate::TaskId::new(20),
    };
    *control = crate::support::control(true, true, state, slot);
    let mut old = tracked(Action::Ruin, Origin::Reaper, Some(crate::TaskId::new(20)));
    let mut explicit = request(Action::Ruin, Origin::External);
    assert_eq!(lifecycle::takeover::accept(&mut old, &mut explicit), Ok(false));
    assert_eq!(old.operation.key.waiters.len(), 2);
    assert_eq!(control.run_requested("worker"), Some(true));
    lifecycle::intent::accepted(control, "worker", Some(explicit.directive));
    assert_eq!(control.run_requested("worker"), Some(false));
}

#[test]
fn merged_ruin_commits_stop_before_and_after_old_pre_and_denies_recovery() {
    reset();
    let mut before = crate::support::control(true, true, State::Ready, Slot::None);
    merge(&mut before, State::Ready);
    let mut after = crate::support::control(true, true, State::Stopping, Slot::None);
    merge(&mut after, State::Stopping);

    let mut retired = control(true, true, State::Dead, Slot::None);
    let mut old = tracked(Action::Ruin, Origin::Rollback, Some(crate::TaskId::new(20)));
    let mut explicit = request(Action::Ruin, Origin::External);
    assert_eq!(
        lifecycle::takeover::accept(&mut old, &mut explicit),
        Ok(false)
    );
    lifecycle::intent::accepted(&mut retired, "worker", Some(explicit.directive));
    assert_eq!(
        admit(retired, active(Action::Recover, recovery(), "worker"), open()),
        Err(Fail::Denied)
    );

    let mut conflict = control(
        true,
        true,
        State::Ready,
        Slot::Live {
            team: None,
            task: crate::TaskId::new(20),
        },
    );
    let mut embark = tracked(
        Action::Embark { parent: None },
        Origin::External,
        Some(crate::TaskId::new(20)),
    );
    let mut rejected = request(Action::Ruin, Origin::External);
    assert_eq!(
        lifecycle::takeover::accept(&mut embark, &mut rejected),
        Err(Fail::NotReady)
    );
    lifecycle::intent::accepted(&mut conflict, "worker", None);
    assert_eq!(conflict.run_requested("worker"), Some(true));

    let mut invalid = control(true, true, State::Dead, Slot::None);
    let mut recovering = tracked(Action::Recover, recovery(), None);
    let mut wrong_origin = request(Action::Ruin, Origin::Startup);
    assert_eq!(
        lifecycle::takeover::accept(&mut recovering, &mut wrong_origin),
        Err(Fail::NotReady)
    );
    lifecycle::intent::accepted(&mut invalid, "worker", Some(wrong_origin.directive));
    assert_eq!(invalid.run_requested("worker"), Some(true));

    let mut full_control = control(
        true,
        true,
        State::Ready,
        Slot::Live {
            team: None,
            task: crate::TaskId::new(20),
        },
    );
    let mut full_waiters = Waiters::with_capacity(1);
    full_waiters.push(sender());
    let mut full = tracked_with_waiters(
        Action::Ruin,
        Origin::Reaper,
        Some(crate::TaskId::new(20)),
        full_waiters,
    );
    let mut too_many = request(Action::Ruin, Origin::External);
    assert_eq!(
        lifecycle::takeover::accept(&mut full, &mut too_many),
        Err(Fail::Full)
    );
    lifecycle::intent::accepted(&mut full_control, "worker", None);
    assert_eq!(full_control.run_requested("worker"), Some(true));
}
