use env::{PieToken, TaskId};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Heir {
    pub(crate) task: TaskId,
    pub(crate) token: PieToken,
}
