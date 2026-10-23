//! Result of one bounded scan call.

use super::end::End;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Page {
    /// First unscanned requester.
    pub next: u32,
    /// Number of `Function` records written.
    pub count: usize,
    /// Why this call stopped.
    pub end: End,
}
