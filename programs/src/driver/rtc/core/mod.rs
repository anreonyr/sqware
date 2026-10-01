//! 不碰内核、不碰设备的那一半。
//! 对外只有三件：Host（会话核）、frame（形与记号）、fail::Fail（失败域）——
//! 那一格是核的内件，它的不变量（一台设备一个闹钟）由 Host 持有它的方式承载。
//! 这一层与 `crates/protocol/src/driver/line/` 同一个分工（那边账与形分住：账在 `programs/src/driver/router/core/lines.rs`，形在 `frame.rs`）：
//! **只有数据与决定**。碰内核的那几手（收发、借孔、推帧）住 super::client；碰内核动作与
//! 设备的那几手住 `src/driver/rtc/adapt/`。

pub mod fail;
pub mod frame;
pub mod host;
mod slot;

pub use fail::Fail;
pub use host::Host;
