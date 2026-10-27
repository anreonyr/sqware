//! 落位：绑定号、总线 requester 与 BAR。接收方是实际调用者。

use env::PieToken;

#[derive(env::Frame, Clone, Copy, PartialEq, Eq, Debug)]
#[frame(len = 24)]
pub struct Place {
    pub op: u8,
    pub serial: u64,
    pub requester: u32,
    pub bar: u8,
    pub back: PieToken,
}
