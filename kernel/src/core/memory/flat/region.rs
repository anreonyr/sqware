use alloc::sync::Arc;
use core::num::NonZeroUsize;

use crate::core::memory::PAGE_SIZE;
use crate::core::memory::address::PhysAddr;

use super::error::FlatError;

/// 调用者保证仍然保留的物理区间。它不冒充分配器拥有的 Frame，
/// 其 Arc 不接管底层物理存储的所有权。
pub struct BorrowedRegion {
    start: PhysAddr,
    size: NonZeroUsize,
}

impl BorrowedRegion {
    /// 登记 [start, start + size) 为借用区间。
    ///
    /// # Safety
    /// 调用者必须保证该物理区间在借用期间不被分配或复用。
    pub unsafe fn new(start: PhysAddr, size: usize) -> Result<Arc<Self>, FlatError> {
        if !start.is_aligned() || size == 0 || size & (PAGE_SIZE - 1) != 0 {
            return Err(FlatError::NotAligned);
        }
        start
            .as_usize()
            .checked_add(size - 1)
            .ok_or(FlatError::NotAligned)?;
        let size = NonZeroUsize::new(size).ok_or(FlatError::NotAligned)?;
        Arc::try_new(Self { start, size }).map_err(|_| FlatError::OutOfMemory)
    }

    pub fn start(&self) -> PhysAddr {
        self.start
    }

    pub fn size(&self) -> usize {
        self.size.get()
    }
}
