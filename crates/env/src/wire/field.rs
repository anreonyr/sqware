//! Byte codecs for environment-local semantic handles, plus shared `wire` reexports.

use crate::wire::{PieToken, TaskId};

pub use ::wire::{Field, Span, fetch_bytes, fetch_tail, store_bytes, store_tail, times, total};

impl Field for TaskId {
    const WIDTH: usize = 8;
    fn store(&self, out: &mut [u8]) {
        out.copy_from_slice(&(self.get() as u64).to_le_bytes());
    }
    fn fetch(bytes: &[u8]) -> Option<Self> {
        let raw: [u8; 8] = bytes.get(..8)?.try_into().ok()?;
        Some(TaskId::new(u64::from_le_bytes(raw) as usize))
    }
}

impl Field for PieToken {
    const WIDTH: usize = 8;
    fn store(&self, out: &mut [u8]) {
        out.copy_from_slice(&self.to_bytes());
    }
    fn fetch(bytes: &[u8]) -> Option<Self> {
        PieToken::from_bytes(bytes.get(..8)?)
    }
}
