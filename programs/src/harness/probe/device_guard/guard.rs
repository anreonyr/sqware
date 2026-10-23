use ::resource::dock::View;
use env::{TaskId, TeamId};

/// The Root-side facts retained across one Guard scene.
pub(super) struct Guard {
    pub uart: View,
    pub target: &'static str,
    pub submitted: bool,
    pub armed: Option<(TaskId, TeamId)>,
    pub ready: bool,
}
