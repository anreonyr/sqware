//! 新 RAM 来源的物理分配约束（过线的那一格）。

use crate::{Decode, Wire};

use super::PAGE_SIZE;

/// 新 RAM 来源：`size` 是字节数（页对齐向上取整），`align` 是每页起址的对齐下界。
///
/// 对齐只约束**每页起址**的下界，不表示各页物理连续；不支持的约束在构造时明确拒绝，
/// 不接受后忽略。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Allocation {
    pub size: usize,
    pub align: usize,
}

impl Allocation {
    /// 过线的字数（[`Wire`] 一进一出都走这两格）。
    pub const WORDS: usize = 2;

    /// 尺寸非零、对齐是 2 的幂，且向上取整到页不溢出。
    pub fn new(size: usize, align: usize) -> Option<Self> {
        if size == 0 || !align.is_power_of_two() {
            return None;
        }
        size.checked_next_multiple_of(PAGE_SIZE)?;
        Some(Self { size, align })
    }
}

impl Wire for Allocation {
    fn pack(&self, s: &mut [usize; 6], i: &mut usize) {
        self.size.pack(s, i);
        self.align.pack(s, i);
    }
    fn unpack(s: &[usize; 6], i: &mut usize) -> Result<Self, Decode> {
        let size = usize::unpack(s, i)?;
        let align = usize::unpack(s, i)?;
        Self::new(size, align).ok_or(Decode::Invalid)
    }
}
