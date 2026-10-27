//! 收进来的一问：六形按动作码分派，长度必须与那一形恰好相等。
//!
//! 64 字节的接收缓冲容纳最长那一形；短截、长短不符或动作码不认识都不猜。

use env::wire::Span as _;

use super::{
    ACQUIRE, Acquire, ESTABLISH, Establish, INVALIDATE, PUBLISH, QUIESCE, RELEASE, Step,
};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Wire {
    Establish(Establish),
    Acquire(Acquire),
    Publish(Step),
    Invalidate(Step),
    Quiesce(Step),
    Release(Step),
}

impl Wire {
    /// 最长那一形的接收缓冲：Acquire 61 字节。
    pub const LEN: usize = 64;

    pub fn take(bytes: &[u8]) -> Option<Self> {
        match *bytes.first()? {
            ESTABLISH => {
                let (value, end) = <Establish as env::wire::Span>::fetch_at(bytes, 0)?;
                (value.op == ESTABLISH && end == bytes.len()).then_some(Self::Establish(value))
            }
            ACQUIRE => {
                let (value, end) = <Acquire as env::wire::Span>::fetch_at(bytes, 0)?;
                (value.op == ACQUIRE && end == bytes.len()).then_some(Self::Acquire(value))
            }
            PUBLISH => step::<{ PUBLISH }>(bytes).map(Self::Publish),
            INVALIDATE => step::<{ INVALIDATE }>(bytes).map(Self::Invalidate),
            QUIESCE => step::<{ QUIESCE }>(bytes).map(Self::Quiesce),
            RELEASE => step::<{ RELEASE }>(bytes).map(Self::Release),
            _ => None,
        }
    }

    pub fn store(&self, out: &mut [u8]) -> Option<usize> {
        match self {
            Self::Establish(value) if value.op == ESTABLISH => value.store_at(out, 0),
            Self::Acquire(value) if value.op == ACQUIRE => value.store_at(out, 0),
            Self::Publish(value) if value.op == PUBLISH => value.store_at(out, 0),
            Self::Invalidate(value) if value.op == INVALIDATE => value.store_at(out, 0),
            Self::Quiesce(value) if value.op == QUIESCE => value.store_at(out, 0),
            Self::Release(value) if value.op == RELEASE => value.store_at(out, 0),
            _ => None,
        }
    }
}

fn step<const OP: u8>(bytes: &[u8]) -> Option<Step> {
    let (value, end) = <Step as env::wire::Span>::fetch_at(bytes, 0)?;
    (value.op == OP && end == bytes.len()).then_some(value)
}

impl ::wire::Message for Wire {
    type In = Self;
    type Buf = [u8; Self::LEN];
    const EMPTY: Self::Buf = [0; Self::LEN];

    fn store(&self, out: &mut [u8]) -> Option<usize> {
        Wire::store(self, out)
    }

    fn fetch(bytes: &[u8]) -> Option<Self> {
        Wire::take(bytes)
    }
}
