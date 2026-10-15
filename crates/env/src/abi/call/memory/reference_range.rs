//! 一份引用上的子范围：句柄 + 相对页偏移 + 页数。
//!
//! 页数非零且坐标不溢出（见 [`pages`](super::pages)）；这是 `Attach` 长度的
//! 唯一来源，选址不另带一份长度。

use crate::wire::Field;
use crate::wire::FlatReference;
use crate::{Decode, Wire};

use super::pages;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ReferenceRange {
    pub reference: FlatReference,
    pub offset: usize,
    pub count: usize,
}

impl ReferenceRange {
    /// 过线的字数；也是它在用户内存里作定长描述时的字数（同一份布局）。
    pub const WORDS: usize = 3;

    pub fn new(reference: FlatReference, offset: usize, count: usize) -> Option<Self> {
        pages(offset, count).ok()?;
        Some(Self {
            reference,
            offset,
            count,
        })
    }

    /// 把自身写进定长描述字；与 [`Wire`] 是同一份布局，不重写第二份。
    pub fn encode(self) -> [usize; Self::WORDS] {
        let mut buffer = [0usize; 6];
        let mut at = 0usize;
        self.pack(&mut buffer, &mut at);
        debug_assert_eq!(at, Self::WORDS);
        let mut words = [0usize; Self::WORDS];
        words.copy_from_slice(&buffer[..Self::WORDS]);
        words
    }

    /// 由定长描述字解码；同一份 checked 口径，非法位 → `Err(Decode)`。
    pub fn decode(words: [usize; Self::WORDS]) -> Result<Self, Decode> {
        let mut buffer = [0usize; 6];
        buffer[..Self::WORDS].copy_from_slice(&words);
        let mut at = 0usize;
        let value = Self::unpack(&buffer, &mut at)?;
        if at != Self::WORDS {
            return Err(Decode::Invalid);
        }
        Ok(value)
    }
}

impl Wire for ReferenceRange {
    fn pack(&self, s: &mut [usize; 6], i: &mut usize) {
        self.reference.pack(s, i);
        self.offset.pack(s, i);
        self.count.pack(s, i);
    }
    fn unpack(s: &[usize; 6], i: &mut usize) -> Result<Self, Decode> {
        let reference = FlatReference::unpack(s, i)?;
        let offset = usize::unpack(s, i)?;
        let count = usize::unpack(s, i)?;
        Self::new(reference, offset, count).ok_or(Decode::Invalid)
    }
}

impl Field for ReferenceRange {
    const WIDTH: usize = Self::WORDS * u64::WIDTH;

    fn store(&self, out: &mut [u8]) {
        super::store_words(self.encode(), out);
    }

    fn fetch(bytes: &[u8]) -> Option<Self> {
        Self::decode(super::fetch_words(bytes)?).ok()
    }
}
