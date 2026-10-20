use env::{PieToken, TaskId};

/// Ordinary device owner and its native liveness sensor.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Owner {
    pub task: TaskId,
    pub sensor: PieToken,
}
