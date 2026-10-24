#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Stage {
    Place,
    Attach,
    Acquire,
    Publish,
    Enable,
    Handoff,
    Active,
    Invalidate,
    Disable,
    Stop,
    Quiesce,
    Release,
    Detach,
    CloseHandoff,
    Unplace,
    Done,
}
