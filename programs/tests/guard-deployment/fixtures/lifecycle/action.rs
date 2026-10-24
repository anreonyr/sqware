#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Action {
    Mint,
    Recover,
    Embark { parent: Option<crate::TaskId> },
    Debark,
    Ruin,
}
