#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum DeploymentSelectionError {
    Unknown,
    EntryMismatch,
}
