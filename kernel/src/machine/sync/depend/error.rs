#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DepInitError {
    OutOfMemory,
    AlreadyInit,
}
