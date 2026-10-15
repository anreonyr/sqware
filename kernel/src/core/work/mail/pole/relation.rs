use env::TeamId;

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum Relation {
    Input,
    Open,
    View,
    Staged(TeamId),
}
