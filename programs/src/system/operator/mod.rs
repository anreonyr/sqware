//! operator::实现侧 — **持树者**与它那本客人账。
//!
//! 与 [`crate::system::board`] 同一个判据：**规范与接口**（正文、判定、帧、客侧三手、装配侧
//! `attach`/`host_of`）住 `crates/protocol/src/system/operator/`；**实现方**（**独立域**，
//! `prog-operator` 那一台 + 它记的账）住这里。

pub mod bridge;
pub mod server;
