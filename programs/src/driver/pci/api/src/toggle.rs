//! 开启／清除 MASTER：绑定号与总线 requester。

use env::PieToken;

#[derive(env::Frame, Clone, Copy, PartialEq, Eq, Debug)]
#[frame(len = 24)]
pub struct Toggle {
    pub op: u8,
    pub serial: u64,
    pub requester: u32,
    pub back: PieToken,
}
