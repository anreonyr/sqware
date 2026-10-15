//! `Detach` / `SetAccess` 的选择：整段，或创建时定下的页坐标窗口。
//!
//! `Pages` 的坐标以**创建时的来源页窗口**为准，不因中间形成空洞而重排；
//! 选到已经关闭的页就是失败。两种选择调同一对入口。

use crate::{Decode, Wire};

use super::pages;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ViewSelection {
    Whole,
    Pages { offset: usize, count: usize },
}

impl ViewSelection {
    /// 过线的字数：标签 + 偏移 + 页数。
    pub const WORDS: usize = 3;

    pub fn pages(offset: usize, count: usize) -> Option<Self> {
        pages(offset, count).ok()?;
        Some(Self::Pages { offset, count })
    }
}

impl Wire for ViewSelection {
    fn pack(&self, s: &mut [usize; 6], i: &mut usize) {
        match *self {
            Self::Whole => {
                0usize.pack(s, i);
                0usize.pack(s, i);
                0usize.pack(s, i);
            }
            Self::Pages { offset, count } => {
                1usize.pack(s, i);
                offset.pack(s, i);
                count.pack(s, i);
            }
        }
    }
    fn unpack(s: &[usize; 6], i: &mut usize) -> Result<Self, Decode> {
        let tag = usize::unpack(s, i)?;
        let offset = usize::unpack(s, i)?;
        let count = usize::unpack(s, i)?;
        match tag {
            0 if offset == 0 && count == 0 => Ok(Self::Whole),
            1 => Self::pages(offset, count).ok_or(Decode::Invalid),
            _ => Err(Decode::Invalid),
        }
    }
}
