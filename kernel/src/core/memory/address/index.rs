/// 来源内的逻辑页号。它与物理地址、页内偏移是不同量纲，不互换、不参与地址运算。
#[repr(transparent)]
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
pub struct PageIndex(usize);

impl PageIndex {
    pub const ZERO: Self = Self(0);

    #[inline]
    pub fn new(value: usize) -> Self {
        Self(value)
    }

    #[inline]
    pub fn get(self) -> usize {
        self.0
    }
}
