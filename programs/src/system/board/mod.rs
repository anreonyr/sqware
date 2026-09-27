//! board::实现侧 — **板那一台**、它那本客人账，与**说话的那一侧**（客侧那几手）。
//!
//! 板这份协议分三层住：**规范与记号**（正文、帧、失败域、那几格记号）住
//! `crates/protocol/src/system/board/`；**客侧**（[`client`]：从外面找上板的那几手）与
//! **实现方**（真在编排域里跑的那枚线程 + 它记的账）住这里（`programs/src/system/board/`）。
//!
//! **照实记（客侧原先住 protocol）**：按裁定「board 是编排域的**死信号传感器**，不是第五轴」，
//! `client.rs` 从 protocol 那一层退回实现侧——那里只留**形与记号**（见 [`client`] 的头注）。
//! 判据仍是同一条：**谁在说话**——从外面找上板的人（客人、装配者）用的一切是那一侧的接口；
//! 板自己怎么站住、怎么记账是实现。

pub mod bridge;
pub mod client;
// **照实记（`core` 是残枝那一刀从 protocol 搬来的）**：板那本账原先住
// `crates/protocol/src/system/board/core.rs`——读者只有本域的持板线程，故回这里。
pub mod core;
pub mod server;
