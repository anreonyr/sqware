use super::Close;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Phase {
    Open,
    Closing(Close),
    Stopped,
}
