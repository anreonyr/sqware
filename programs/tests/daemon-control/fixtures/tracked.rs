use super::Operation;

pub struct Tracked {
    pub operation: Operation,
    pub cursor: ::schedule::Cursor,
    pub complete: bool,
}
