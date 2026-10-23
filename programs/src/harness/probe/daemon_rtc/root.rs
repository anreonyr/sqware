use env::{TaskId, TeamId};

/// Root-side facts retained while the production app frame runs one RTC recovery.
pub(super) struct Root {
    pub controller: Option<TaskId>,
    pub old: Option<(TaskId, TeamId)>,
    pub new: Option<(TaskId, TeamId)>,
    pub retired: bool,
    pub killed: bool,
}

impl Root {
    pub(super) fn new() -> Self {
        Self {
            controller: None,
            old: None,
            new: None,
            retired: false,
            killed: false,
        }
    }
}
