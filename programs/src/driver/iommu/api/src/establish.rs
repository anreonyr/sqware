//! 建立或取回一个持久 Domain：requester 由实际 Driver 在门外给出。

use env::PieToken;

/// req 13 字节；回信孔仍是原样那一枚。
#[derive(env::Frame, Clone, Copy, PartialEq, Eq, Debug)]
#[frame(len = 13)]
pub struct Establish {
    pub op: u8,
    pub requester: u32,
    pub back: PieToken,
}
