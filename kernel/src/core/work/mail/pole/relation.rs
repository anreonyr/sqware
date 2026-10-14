use env::TeamId;

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum Relation {
    Input,
    Keeper,
    Authority,
    Open,
    View,
    Staged(TeamId),
}
