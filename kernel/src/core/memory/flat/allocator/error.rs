use fack::prelude::Error;

#[derive(Error, Debug, Clone, Copy, PartialEq, Eq)]
#[allow(unused)]
pub enum InitError {
    #[error("no free memory region configured")]
    NoFreeMemory,
    #[error("memory allocation failed while initializing allocator")]
    OutOfMemory,
    #[error("no free physical frames available")]
    NoFreeFrames,
    #[error("no harts reported")]
    NoHarts,
    #[error("allocator already initialized")]
    AlreadyInitialized,
}

pub type InitResult<T> = erra::Result<T, InitError>;
