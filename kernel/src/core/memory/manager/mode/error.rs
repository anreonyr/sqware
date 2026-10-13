#[derive(fack::prelude::Error, Debug, Clone, Copy, PartialEq, Eq)]
pub enum SatpError {
    #[error("no supported page table mode")]
    Unsupported,
    #[error("page table mode probe ran out of memory")]
    OutOfMemory,
}
