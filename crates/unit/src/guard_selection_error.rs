#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum GuardSelectionError {
    Unknown,
    Duplicate,
    Unstarted,
}
