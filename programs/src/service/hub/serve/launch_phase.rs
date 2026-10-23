#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum LaunchPhase {
    Inspect,
    Embark,
    Return { trusted: bool },
}
