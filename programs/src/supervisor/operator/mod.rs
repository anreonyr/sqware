//! operator::实现侧 — **持树者**与它那本客人账。
//!
//! 与 [`crate::supervisor::system::board`] 同一个判据：**规范与接口**（正文、判定、帧、客侧三手、装配侧
//! `attach`/`host_of`）住 `crates/protocol/src/operator/`；**实现方**（真在 `prog-operator`
//! 域里跑的那枚线程 + 它记的账）住这里。

pub mod bridge;
pub mod desk;
pub mod fail;
pub mod server;
