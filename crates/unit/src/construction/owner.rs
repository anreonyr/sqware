#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Owner {
    Caller,
    Selected,
    Exact(env::TaskId),
    NamedUnit(&'static str),
}
