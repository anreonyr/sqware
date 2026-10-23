//! Bounded read-only access to one mapped ECAM window.

pub const MAX_FUNCTIONS: u32 = u16::MAX as u32 + 1;

/// A source of configuration bytes with an exclusive function limit.
///
/// `limit` is the first requester id that is outside the mapped window. It is
/// never larger than the 65536-BDF address space; implementations may be
/// smaller when the mapping is partial.
pub trait Reader {
    fn limit(&self) -> u32;

    /// Reads one byte at `register` for `requester`.
    ///
    /// `register` is an offset inside the function's 4 KiB configuration
    /// space. Out-of-window requests return `None`; they must not perform MMIO.
    fn read8(&self, requester: u16, register: u16) -> Option<u8>;
}
