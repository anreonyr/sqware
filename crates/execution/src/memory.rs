//! User memory allocation, mapping and the program heap.

pub use env::PAGE_SIZE;
use env::{MemoryResult, PieToken, TeamId, VirtAddr};

#[path = "memory/allocator.rs"]
mod allocator;

/// Allocate page-rounded memory in the current domain.
pub fn allocate(size: usize) -> MemoryResult<usize> {
    env::memory::allocate(size).map(|address| address.get())
}

/// Release an allocation using its exact original size.
pub fn deallocate(address: usize, size: usize) -> MemoryResult<()> {
    env::memory::deallocate(VirtAddr::new(address), size)
}

/// Install a mapping in a domain.
pub fn map(
    team: TeamId,
    at: usize,
    size: usize,
    backing: PieToken,
    offset: usize,
    flags: u64,
) -> MemoryResult<usize> {
    env::memory::mmap(team, VirtAddr::new(at), size, backing, offset, flags)
        .map(|address| address.get())
}

/// Release a mapping or declared region in the current domain.
pub fn munmap(address: usize, size: usize) -> MemoryResult<()> {
    env::memory::munmap(TeamId::new(0), VirtAddr::new(address), size)
}
