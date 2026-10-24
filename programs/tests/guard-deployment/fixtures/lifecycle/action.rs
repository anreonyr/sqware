#[derive(Clone, Copy)]
pub enum Action {
    Embark { parent: Option<crate::TaskId> },
    Ruin,
}
