use super::{Action, Execution, Origin};
use env::TaskId;

/// `false` means a successful recovery Mint still has to run its Embark phase
/// before the caller may receive success.
pub(crate) fn completes(action: Action, origin: Origin) -> bool {
    !(matches!(action, Action::Recover) && matches!(origin, Origin::Recovery { .. }))
}

/// Whether a failed action still owns a constructed Team and must roll back
/// through Ruin. Recovery keeps its obligation even after Mint moved the
/// Service into the pending queue (`instance` is then `None`).
pub(crate) fn rollback(action: Action, origin: Origin, execution: &Execution) -> bool {
    matches!(
        action,
        Action::Mint | Action::Recover | Action::Embark { .. }
    ) && (execution.instance.is_some()
        || (matches!(origin, Origin::Recovery { .. }) && execution.task.is_some()))
}

/// A failed Ruin that still owns a task keeps the job and stops the loop.
pub(crate) fn cleanup_failure(action: Action, task: Option<TaskId>) -> bool {
    matches!(action, Action::Ruin) && task.is_some()
}
