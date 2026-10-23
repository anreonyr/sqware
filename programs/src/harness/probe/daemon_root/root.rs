use env::{PieToken, TaskId, TeamId};

/// Root-side facts retained while the production app frame runs one recovery.
pub(super) struct Root {
    pub controller: Option<TaskId>,
    pub old: Option<(TaskId, TeamId)>,
    pub new: Option<(TaskId, TeamId)>,
    pub retired: bool,
    pub killed: bool,
    pub ack: Option<PieToken>,
    pub deadline: u64,
    pub reaped: bool,
    pub queued: bool,
    pub ack_sent: bool,
}

impl Root {
    pub(super) fn new() -> Self {
        Self {
            controller: None,
            old: None,
            new: None,
            retired: false,
            killed: false,
            ack: None,
            deadline: 0,
            reaped: false,
            queued: false,
            ack_sent: false,
        }
    }
}
