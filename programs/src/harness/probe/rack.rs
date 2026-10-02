//! 架那三台探针的**共用件**：那条试验场的路、那一批名字、那一条载荷。
//!
//! 与 [`super::count`] 同位（探针那一档的共享件住本目录顶层、由各台独立 bin 各自 `use` 一次）：
//! 本件给的是三台之间**必须一致**的那几样，不是量法本身。
//!
//! - `probe-rack`（单域）：量**队列语义与唤醒协议**（不需要树，也不需要第二域）。
//! - `probe-rack-mount`（铺场那一侧）：开两具架、落**两枚**砖（每枚后面是一具完整的架）、把 A 写满、响 `Ready`。
//! - `probe-rack-guest`（对端那一侧）：用**产品同一条** [`client::find`](crate::driver::uart::client::find)
//!   取回两端——故真机量到的就是客人这条路本身。
//!
//! **两具架的名字与产品那两条同形**：`RX` = "对面写、本端读"（铺场写、客人读），
//! `TX` = "本端写、对面读"（客人写、铺场读）。名字本身借 `core::frame` 那一处。

use env::PieToken;
use protocol::common::path::PathBuf;
use protocol::communication::rack::{CAP, Rack};

use crate::driver::uart::core::frame::{self, Bytes};

/// 试验场那条路：**测具自己的**一格（不走 `/svc/drv`——那边是产品的面）。
pub const ROAD: &str = "probe-rack";

/// 两侧各要发几条：**正好一具架的容量**（不绕环 ⇒ 读数确定，丢不丢都看得出）。
pub fn count() -> usize {
    CAP
}

/// 那一条载荷：`i` 的四字节小端。**两侧同一手**，故"读到的与落下的对不上"只有一种解释。
pub fn payload(i: usize) -> Bytes {
    match Bytes::of(&(i as u32).to_le_bytes()) {
        Some(one) => one,
        None => panic!("probe-rack: payload"),
    }
}

/// 两块窗格（A：铺场写／客人读；B：客人写／铺场读）在树上的**两枚砖**——**每个方向一枚**：
/// 那一枚后面就是一具完整的架（页上那一位即铃），故页与铃不各占一格。
///
/// 名字借 `core::frame` 那一处（与产品同一批），故"哪一枚是哪一端"不必在两台里各说一遍。
pub fn faces(a: &Rack<Bytes>, b: &Rack<Bytes>) -> [(&'static str, PieToken); 2] {
    [(frame::RX, a.ship()), (frame::TX, b.ship())]
}

/// 试验场那条路的那一枚 `PathBuf`。
pub fn road() -> Option<PathBuf> {
    PathBuf::try_new(ROAD)
}
