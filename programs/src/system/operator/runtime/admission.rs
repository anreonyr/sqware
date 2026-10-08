//! Request admission uses the native caller, never a caller-selected capability label.

use env::TaskId;
use system_api::operator::Wire;

pub(super) fn caller(who: TaskId, control: TaskId, request: &Wire) -> bool {
    !matches!(
        request,
        Wire::Part { .. } | Wire::Land { .. } | Wire::Trim(_)
    ) || who == control
}
