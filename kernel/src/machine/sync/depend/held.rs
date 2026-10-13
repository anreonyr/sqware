use super::level::Level;

#[derive(Clone, Copy)]
pub(super) struct Held {
    pub(super) addr: usize,
    pub(super) level: Option<Level>,
    pub(super) caller: usize,
}
