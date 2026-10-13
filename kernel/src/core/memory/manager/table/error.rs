use fack::prelude::Error;

#[derive(Error, Debug, Clone, Copy, PartialEq, Eq)]
pub enum MapError {
    #[error("physical frame allocator exhausted")]
    OutOfMemory,
    #[error("virtual address already mapped")]
    AlreadyMapped,
    #[error("address not page-aligned")]
    NotAligned,
    #[error("page table entry not mapped")]
    NotMapped,
    #[error("virtual address not in any declared map")]
    NoRegion,
    #[error("DRAM identity map exceeds the lower address half")]
    DramOverlap,
    #[error("cannot widen a borrowed mapping")]
    WidenDenied,
    #[error("address-space shootdown failed")]
    Shootdown,
    #[error("kernel address space has permanent lifetime")]
    Pinned,
}
