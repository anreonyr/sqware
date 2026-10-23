use crate::support::{active, control, open, recovery, recheck, reset, tracked};
use crate::system::control::lifecycle::{self, Action, Fail, Origin};
use crate::system::control::unit::table::{Slot, State};
use crate::system::app::Fault;
use schedule::Progress;

#[test]
fn recovery_finish_transitions_to_embark_and_rolls_back_without_instance() {
    reset();
    let mut recovery_job = tracked(Action::Recover, recovery(), None);
    assert_eq!(recovery_job.finish(Ok(Progress::Done)), Ok(()));
    assert!(!recovery_job.complete);
    assert_eq!(
        recovery_job.operation.key.request.directive.action,
        Action::Embark { parent: None }
    );
    assert_eq!(
        recovery_job.operation.key.request.directive.origin,
        recovery()
    );
    assert!(recovery_job.operation.key.request.back.is_some());
    assert_eq!(crate::ANSWERS.with(|answers| answers.borrow().len()), 0);
    assert_eq!(
        lifecycle::intent::change(
            recovery_job.operation.key.request.directive.action,
            recovery_job.operation.key.request.directive.origin,
        ),
        None
    );

    let minted = control(
        true,
        true,
        State::Starting,
        Slot::Live {
            team: None,
            task: crate::TaskId::new(20),
        },
    );
    let launch = active(
        Action::Embark { parent: None },
        recovery(),
        "worker",
    );
    assert_eq!(recheck(minted, launch, open()), Ok(Progress::Done));

    assert_eq!(recovery_job.finish(Ok(Progress::Done)), Ok(()));
    assert!(recovery_job.complete);
    assert!(recovery_job.operation.key.request.back.is_some());
    assert_eq!(crate::ANSWERS.with(|answers| answers.borrow().len()), 0);

    let mut rolled = tracked(
        Action::Recover,
        recovery(),
        Some(crate::TaskId::new(20)),
    );
    assert!(rolled.operation.progress.execution.instance.is_none());
    assert_eq!(
        rolled.finish(Err(Fail::NotReady)),
        Ok(())
    );
    assert!(!rolled.complete);
    assert_eq!(rolled.operation.key.request.directive.action, Action::Ruin);
    assert_eq!(
        rolled.operation.key.request.directive.origin,
        Origin::Rollback
    );
    assert_eq!(rolled.operation.progress.failure, Some(Fail::NotReady));
    assert_eq!(
        rolled.operation.progress.execution.deadline,
        100 + 3000 * 1_000_000
    );

    let mut failed_ruin = tracked(
        Action::Ruin,
        Origin::External,
        Some(crate::TaskId::new(20)),
    );
    assert_eq!(
        failed_ruin.finish(Err(Fail::NotReady)),
        Err(Fault::Shutdown)
    );
}
