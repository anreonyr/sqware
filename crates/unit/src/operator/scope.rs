#[derive(Clone, Copy)]
pub enum Scope {
    Subtree,
    Names(&'static [&'static str]),
}
