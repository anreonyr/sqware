/// 来源范围内的页偏移。相对于某个范围起点，不能与逻辑页号互换。
#[repr(transparent)]
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
pub struct PageOffset(usize);

impl PageOffset {
    /// 窗口的第一页；健康用例按它取物化结果。
    #[allow(dead_code)]
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
