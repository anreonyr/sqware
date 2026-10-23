#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Fail {
    Unknown,
    Full,
    NotReady,
}
