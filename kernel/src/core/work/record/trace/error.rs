use fack::prelude::Error;

#[derive(Error, Debug, Clone, Copy, PartialEq, Eq)]
pub enum TraceInitError {
    #[error("spare ring allocation failed")]
    OutOfMemory,
    #[error("trace already initialized")]
    AlreadyInit,
}
