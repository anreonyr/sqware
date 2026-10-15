#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Failure {
    Unknown,
    Conflict,
    Closed,
    Full,
    Denied,
    Unavailable,
    Pending,
}
