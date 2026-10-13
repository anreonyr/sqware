use alloc::sync::Arc;
use core::num::NonZeroUsize;

use crate::core::memory::PAGE_SIZE;
use crate::core::memory::manager::addr::PhysAddr;
use crate::core::memory::manager::table::MapError;

/// 调用者保证仍然保留的物理区间。它不冒充分配器拥有的 Frame。
pub struct BorrowedRegion {
    pub(super) start: PhysAddr,
    pub(super) size: NonZeroUsize,
}

impl BorrowedRegion {
    /// 登记 [start, start + size) 为借用区间。
    ///
    /// # Safety
    /// 调用者必须保证该物理区间在借用期间不被分配或复用。
    pub unsafe fn new(start: PhysAddr, size: usize) -> Result<Arc<Self>, MapError> {
        if !start.is_aligned() || size == 0 || size & (PAGE_SIZE - 1) != 0 {
            return Err(MapError::NotAligned);
        }
        start
            .as_usize()
            .checked_add(size - 1)
            .ok_or(MapError::NotAligned)?;
        let size = NonZeroUsize::new(size).ok_or(MapError::NotAligned)?;
        Arc::try_new(Self { start, size }).map_err(|_| MapError::OutOfMemory)
    }

    pub fn size(&self) -> usize {
        self.size.get()
    }
}
