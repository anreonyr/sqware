use ::resource::dock::View;
use env::{TaskId, TeamId};

/// The Root-side facts retained across one Guard scene.
pub(super) struct Guard {
    pub uart: View,
    pub submitted: bool,
    pub armed: Option<(&'static str, TaskId, TeamId)>,
    pub ready: bool,
}
