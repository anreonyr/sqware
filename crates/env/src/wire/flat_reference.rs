//! 一份来源持有在**持有它的那张表**里的句柄（0 = 无效哨兵）。
//!
//! 与 [`PieToken`](super::PieToken) 同族：号只在自己那张表里解析得动，故本类型也
//! `!Send + !Sync`——跨线程只剩「交出裸值、对端另落一枚」这一条路。不装内核指针。

use core::marker::PhantomData;

use super::{Decode, Wire};

#[repr(transparent)]
#[derive(Clone, Copy, PartialEq, Eq, Debug, PartialOrd, Ord, Hash)]
pub struct FlatReference(pub(crate) usize, PhantomData<*const ()>);

impl FlatReference {
    /// 无效哨兵（0）：没有这一份持有。
    pub const NONE: Self = Self(0, PhantomData);

    /// 号在**线上**的宽度（8 字节 LE）——与 [`PieToken`](super::PieToken) 同形。
    pub const WIDTH: usize = 8;

    /// 由裸值造一枚（线上解码面）。
    pub(crate) const fn new(v: usize) -> Self {
        Self(v, PhantomData)
    }

    /// **收号**：运行时要记住一枚句柄再交回去（失败重试），走的就是这扇
    /// "收到一枚号"的门；伪造的号在自己那张表里查不到 ⇒ 照旧 Denied。
    pub fn from_bytes(bytes: &[u8]) -> Option<Self> {
        let at = bytes.get(..Self::WIDTH)?;
        let raw = u64::from_le_bytes(at.try_into().ok()?);
        let raw = usize::try_from(raw).ok()?;
        (raw <= isize::MAX as usize).then(|| Self::new(raw))
    }

    /// **交出**：号变回线上的那 8 字节（与 [`FlatReference::from_bytes`] 成对）。
    pub fn to_bytes(self) -> [u8; Self::WIDTH] {
        (self.0 as u64).to_le_bytes()
    }

    pub const fn get(self) -> usize {
        self.0
    }
}

impl From<FlatReference> for usize {
    fn from(reference: FlatReference) -> Self {
        reference.0
    }
}

impl Wire for FlatReference {
    fn pack(&self, s: &mut [usize; 6], i: &mut usize) {
        s[*i] = self.0;
        *i += 1;
    }
    fn unpack(s: &[usize; 6], i: &mut usize) -> Result<Self, Decode> {
        let v = *s.get(*i).ok_or(Decode::Overflow)?;
        *i += 1;
        if v > isize::MAX as usize {
            return Err(Decode::Invalid);
        }
        Ok(Self::new(v))
    }
}
