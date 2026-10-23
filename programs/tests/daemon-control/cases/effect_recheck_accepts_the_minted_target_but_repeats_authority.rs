use crate::support::{active, control, open, recheck, recovery, reset};
use crate::system::control::lifecycle::{self, Action, Fail};
use crate::system::control::unit::table::{Slot, State};
use schedule::Progress;

fn launch_authority() -> Result<(), Fail> {
    let control = control(
        true,
        true,
        State::Starting,
        Slot::Live {
            team: None,
            task: crate::TaskId::new(20),
        },
    );
    let active = active(
        Action::Embark { parent: None },
        recovery(),
        "worker",
    );
    lifecycle::authority::verify_effect(&active, &control, &open())
}

#[test]
fn effect_recheck_accepts_the_minted_target_but_repeats_authority() {
    reset();
    assert_eq!(launch_authority(), Ok(()));

    let minted = || {
        control(
            true,
            true,
            State::Starting,
            Slot::Live {
                team: None,
                task: crate::TaskId::new(20),
            },
        )
    };
    let launch = || {
        active(
            Action::Embark { parent: None },
            recovery(),
            "worker",
        )
    };
    assert_eq!(recheck(minted(), launch(), open()), Ok(Progress::Done));

    crate::DEAD.with(|dead| dead.borrow_mut().push(crate::TaskId::new(10)));
    assert_eq!(recheck(minted(), launch(), open()), Err(Fail::Denied));
    assert_eq!(launch_authority(), Err(Fail::Denied));
    reset();
    let mut stopped = minted();
    stopped.table.set_run_requested("worker", false).unwrap();
    assert_eq!(recheck(stopped, launch(), open()), Err(Fail::Denied));
    assert_eq!(
        recheck(
            minted(),
            launch(),
            crate::system::app::policy::Settle { closing: true },
        ),
        Err(Fail::Denied)
    );
}

#[test]
fn pre_build_recheck_repeats_intent_caller_and_closing_before_construction() {
    reset();
    let candidate = || control(true, true, State::Dead, Slot::None);
    let build = || active(Action::Recover, recovery(), "worker");
    assert_eq!(recheck(candidate(), build(), open()), Ok(Progress::Done));

    let mut stopped = candidate();
    stopped.table.set_run_requested("worker", false).unwrap();
    assert_eq!(recheck(stopped, build(), open()), Err(Fail::Denied));

    crate::DEAD.with(|dead| dead.borrow_mut().push(crate::TaskId::new(10)));
    assert_eq!(recheck(candidate(), build(), open()), Err(Fail::Denied));
    reset();
    assert_eq!(
        recheck(
            candidate(),
            build(),
            crate::system::app::policy::Settle { closing: true },
        ),
        Err(Fail::Denied)
    );
}
