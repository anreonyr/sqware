use env::TaskId;

#[derive(Clone, Copy)]
pub(super) enum Slot {
    Free,
    Plotting,
    Live {
        address: usize,
        span: usize,
        view: usize,
        pending: Option<(TaskId, usize)>,
    },
    Owed {
        address: usize,
        span: usize,
        view: usize,
        pending: Option<(TaskId, usize)>,
    },
    Closing {
        pending: Option<(TaskId, usize)>,
    },
    Done {
        pending: Option<(TaskId, usize)>,
    },
}
