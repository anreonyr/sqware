use crate::support::{recovery, request, reset, tracked};
use crate::system::control::lifecycle::{self, Action, Origin};
use system_api::control::frame;

#[test]
fn explicit_ruin_takes_over_recovery_before_and_after_mint_without_dropping_the_team() {
    reset();
    let mut before = tracked(Action::Recover, recovery(), None);
    let mut ruin = request(Action::Ruin, Origin::External);
    assert_eq!(lifecycle::takeover::accept(&mut before, &mut ruin), Ok(true));
    assert_eq!(
        before.operation.key.request.directive.action,
        Action::Ruin
    );
    assert_eq!(
        before.operation.key.request.directive.origin,
        Origin::External
    );
    assert_eq!(before.operation.progress.execution.task, None);
    assert!(before.operation.key.request.back.is_some());
    assert_eq!(
        before.operation.progress.execution.deadline,
        100 + 3000 * 1_000_000
    );
    assert_eq!(crate::ANSWERS.with(|answers| answers.borrow().len()), 2);
    crate::ANSWERS.with(|answers| {
        for answer in answers.borrow().iter() {
            assert_eq!(
                answer.status,
                frame::fail_to_code(Some(system_api::control::Fail::NotReady))
            );
        }
    });

    reset();
    let mut after = tracked(
        Action::Embark { parent: None },
        recovery(),
        Some(crate::TaskId::new(20)),
    );
    let mut ruin = request(Action::Ruin, Origin::External);
    assert_eq!(lifecycle::takeover::accept(&mut after, &mut ruin), Ok(true));
    assert_eq!(
        after.operation.progress.execution.task,
        Some(crate::TaskId::new(20))
    );
    assert!(after.operation.progress.execution.instance.is_none());
}
