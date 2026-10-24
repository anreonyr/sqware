use env::TaskId;

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum Mode {
    Dormant,
    Armed { owner: TaskId },
    Tripped,
}
