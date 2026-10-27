//! 一答：状态 ＋ 落位那一枚的坐标。
//!
//! `bar` 是实际调用者表里的精确 BAR token。
//! 不解释它、也不把它当自己表里的号。

use env::PieToken;
use env::wire::Span as _;
use wire::Message;

#[derive(env::Frame, Clone, Copy, PartialEq, Eq, Debug)]
#[frame(len = 16)]
pub struct Said {
    pub status: u8,
    pub pages: u32,
    pub bar: PieToken,
}

impl Said {
    pub const NONE: Self = Self {
        status: 0,
        pages: 0,
        bar: PieToken::NONE,
    };

    pub const fn of(status: u8) -> Self {
        Self {
            status,
            ..Self::NONE
        }
    }
}

impl Message for Said {
    type In = Self;
    type Buf = [u8; Self::LEN];
    const EMPTY: Self::Buf = [0; Self::LEN];

    fn store(&self, out: &mut [u8]) -> Option<usize> {
        self.store_at(out, 0)
    }

    fn fetch(bytes: &[u8]) -> Option<Self> {
        let (value, end) = <Self as env::wire::Span>::fetch_at(bytes, 0)?;
        (end == bytes.len()).then_some(value)
    }
}
