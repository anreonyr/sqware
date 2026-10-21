#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Boundary {
    Found,
    Admit,
    Activate,
    Door,
    Part,
    Bind,
}
