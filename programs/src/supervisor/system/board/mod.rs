//! board::实现侧 — **板那一台**与它那本客人账。
//!
//! 板这份协议分三层住：**规范与接口**（正文、判定、帧、客侧那三手、装配侧那把 `attach`）
//! 住 `crates/protocol/src/system/board/`；**实现方**（真在编排域里跑的那枚线程 + 它记的账）
//! 住这里（`programs/src/supervisor/system/board/`）。
//! 判据只有一条：**谁在说话**——从外面找上板的人（客人、装配者）用的一切是接口；板自己
//! 怎么站住、怎么记账是实现。

pub mod bridge;
pub mod desk;
pub mod server;
