use crate::support::{request, reset, sender, tracked, tracked_with_waiters};
use crate::fixtures::Waiters;
use crate::system::control::lifecycle::{self, Action, Origin};

#[test]
fn duplicate_ruin_merges_and_nonrecovery_conflicts_keep_the_request() {
    reset();
    let mut ruin = tracked(Action::Ruin, Origin::External, Some(crate::TaskId::new(20)));
    let mut next = request(Action::Ruin, Origin::External);
    assert_eq!(lifecycle::takeover::accept(&mut ruin, &mut next), Ok(false));
    assert_eq!(ruin.operation.key.waiters.len(), 2);
    assert_eq!(crate::ANSWERS.with(|answers| answers.borrow().len()), 0);

    let mut embark = tracked(
        Action::Embark { parent: None },
        Origin::External,
        Some(crate::TaskId::new(20)),
    );
    let mut reject = request(Action::Ruin, Origin::External);
    assert_eq!(
        lifecycle::takeover::accept(&mut embark, &mut reject),
        Err(lifecycle::Fail::NotReady)
    );
    assert!(reject.back.is_some());

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
        Err(lifecycle::Fail::Full)
    );
    assert!(too_many.back.is_some());
}
