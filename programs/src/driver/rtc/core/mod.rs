//! rtc::core — **纯功能**：不碰内核、不碰设备的那一半。
//!
//! ```text
//!   host.rs   常驻会话核：事件 → 决定（一问怎么答 / 到点做什么）＋ 那一格 ＋ 读数格
//!   slot.rs   那一格（空着 / 武装着）——核的内件
//!   fail.rs   两个原语会失败在哪一格（**上线**那一格）
//!   frame.rs  形与记号：码 / 五张表 / 解出来的一问 / 报文那一半
//! ```
//!
//! 对外只有三件：[`Host`]（会话核）、[`frame`]（形与记号）、[`fail::Fail`]（失败域）——
//! 那一格是核的内件，它的不变量（一台设备一个闹钟）由 [`Host`] 持有它的方式承载。
//!
//! 这一层与 `crates/contract/driver/line/` 同一个分工（那边是 `core.rs` 账 ＋ `frame.rs` 形）：
//! **只有数据与决定**。碰内核的那几手（船台、借孔、推帧）住 [`super::client`]；碰内核动作与
//! 设备的那几手住 `src/driver/rtc/adapt/`。

pub mod fail;
pub mod frame;
pub mod host;
mod slot;

pub use fail::Fail;
pub use host::Host;
