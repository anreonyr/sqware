use env::TaskId;

#[derive(Clone, Copy)]
pub(super) enum Slot {
    Free,
    Plotting {
        owner: TaskId,
    },
    Live {
        owner: TaskId,
        address: usize,
        view: usize,
        pending: Option<(TaskId, usize)>,
    },
    Owed {
        owner: TaskId,
        address: usize,
        view: usize,
        pending: Option<(TaskId, usize)>,
    },
    Closing {
        owner: TaskId,
        pending: Option<(TaskId, usize)>,
    },
    Done {
        owner: TaskId,
        pending: Option<(TaskId, usize)>,
    },
}

impl Slot {
    pub(super) fn owner(self) -> Option<TaskId> {
        match self {
            Self::Free => None,
            Self::Plotting { owner }
            | Self::Live { owner, .. }
            | Self::Owed { owner, .. }
            | Self::Closing { owner, .. }
            | Self::Done { owner, .. } => Some(owner),
        }
    }
}
