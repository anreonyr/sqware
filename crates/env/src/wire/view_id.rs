//! 目标地址空间里一段关系的句柄（0 = 无效哨兵）。
//!
//! 与 [`FlatReference`](super::FlatReference) 同族，但定位还多一层约束：号只在
//! **目标地址空间那张表**里解析得动，所以每次定位都必须同时给出目标域。
//! 数字身份可跨线程传递；它本身不授予控制权。

use super::{Decode, Wire};

#[repr(transparent)]
#[derive(Clone, Copy, PartialEq, Eq, Debug, PartialOrd, Ord, Hash)]
pub struct ViewId(pub(crate) usize);

impl ViewId {
    /// 无效哨兵（0）：没有这一段关系。
    pub const NONE: Self = Self(0);

    /// 号在**线上**的宽度（8 字节 LE）——与 [`FlatReference`](super::FlatReference)
    /// 同形：一个号在账上占多宽是该号自己的事实。
    pub const WIDTH: usize = 8;

    /// 由裸值造一枚（线上解码面）。
    pub(crate) const fn new(v: usize) -> Self {
        Self(v)
    }

    /// **收号**：号从线上/账上到来时解开它（运行时要记住一个号再交回去，
    /// 走的就是这扇"收到一枚号"的门；伪造的号在目标域那张表里查不到 ⇒ 照旧 Denied）。
    pub fn from_bytes(bytes: &[u8]) -> Option<Self> {
        let at = bytes.get(..Self::WIDTH)?;
        let raw = u64::from_le_bytes(at.try_into().ok()?);
        let raw = usize::try_from(raw).ok()?;
        (raw <= isize::MAX as usize).then(|| Self::new(raw))
    }

    /// **交出**：号变回线上的那 8 字节（与 [`ViewId::from_bytes`] 成对）。
    pub fn to_bytes(self) -> [u8; Self::WIDTH] {
        (self.0 as u64).to_le_bytes()
    }

    pub const fn get(self) -> usize {
        self.0
    }
}

impl From<ViewId> for usize {
    fn from(view: ViewId) -> Self {
        view.0
    }
}

impl Wire for ViewId {
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
