#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Error {
    AlreadyInitialized,
    OutOfMemory,
}
