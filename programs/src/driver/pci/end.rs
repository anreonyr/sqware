//! Why one bounded scan call stopped.

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum End {
    /// The output array was full and the actual limit was not reached.
    Capacity,
    /// The per-call probe budget was exhausted before the actual limit.
    WorkLimit,
    /// The actual mapped-function limit was reached.
    Eof,
}
