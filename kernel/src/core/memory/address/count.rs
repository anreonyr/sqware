use core::num::NonZeroUsize;

use crate::core::memory::PAGE_SIZE;

/// 页数。非零：空范围没有单独表示，不能与"未声明"混为一谈。
#[repr(transparent)]
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
pub struct PageCount(NonZeroUsize);

impl PageCount {
    /// 零不是合法的页数。
    #[inline]
    pub fn new(value: usize) -> Option<Self> {
        NonZeroUsize::new(value).map(Self)
    }

    #[inline]
    pub fn get(self) -> usize {
        self.0.get()
    }

    /// 页数折算字节；溢出返回 `None`。
    #[inline]
    pub fn checked_bytes(self) -> Option<usize> {
        self.0.get().checked_mul(PAGE_SIZE)
    }
}
