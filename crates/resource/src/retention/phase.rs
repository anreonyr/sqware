use super::Close;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Phase {
    Open,
    Stopping(Close),
    Stopped,
    Released,
}
