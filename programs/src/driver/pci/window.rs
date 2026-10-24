//! Bounded read-only view of one granted ECAM mapping.

use core::ptr::read_volatile;
use core::marker::PhantomData;

use super::reader::{Reader, MAX_FUNCTIONS};

pub const FUNCTION_SIZE: usize = 4096;

/// An actual mapped ECAM range. `limit` is derived from `size`, so a short or
/// partial mapping never causes reads outside the granted bytes.
pub struct EcamWindow {
    base: usize,
    size: usize,
    local: PhantomData<*mut ()>,
}

impl EcamWindow {
    /// `size` is the mapped byte length from the actual dock view. Zero-sized
    /// and overflowing ranges and unaligned bases are rejected.
    ///
    /// # Safety
    /// The caller must keep the entire range mapped and readable as ECAM for
    /// every use of this window.
    pub unsafe fn new(base: usize, size: usize) -> Option<Self> {
        if size == 0 || base % FUNCTION_SIZE != 0 {
            return None;
        }
        base.checked_add(size)?;
        Some(Self { base, size, local: PhantomData })
    }

    pub const fn limit(&self) -> u32 {
        let limit = self.size / FUNCTION_SIZE;
        if limit > MAX_FUNCTIONS as usize { MAX_FUNCTIONS } else { limit as u32 }
    }

    pub fn read8(&self, requester: u16, register: u16) -> Option<u8> {
        if u32::from(requester) >= self.limit() || usize::from(register) >= FUNCTION_SIZE {
            return None;
        }
        let offset = usize::from(requester)
            .checked_mul(FUNCTION_SIZE)?
            .checked_add(usize::from(register))?;
        if offset >= self.size {
            return None;
        }
        let at = self.base.checked_add(offset)?;
        // SAFETY: `offset` was checked against the actual mapped `size`, and
        // this type exposes no write operation.
        Some(unsafe { read_volatile(at as *const u8) })
    }
}

impl Reader for EcamWindow {
    fn limit(&self) -> u32 {
        EcamWindow::limit(self)
    }

    fn read8(&self, requester: u16, register: u16) -> Option<u8> {
        EcamWindow::read8(self, requester, register)
    }
}
