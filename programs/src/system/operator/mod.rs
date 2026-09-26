//! operator::实现侧 — **持树者**与它那本客人账。
//!
//! 与 [`crate::system::board`] 同一个判据：**规范与接口**（正文、判定、帧、客侧三手、装配侧
//! `attach`/`host_of`）住 `crates/protocol/src/system/operator/`；**实现方**（iii 之后是**编排域里的一枚线程**（`Role::Tree`）+ 它记的账）住这里——
//! 原先它真在 `prog-operator` 那个域里跑。
//!
//! **照实记（`fail.rs` 已并掉）**：这里原有一份 `fail.rs`（`serve()` 的错误类型），而名册 /
//! 盟册那两台各有一份**同构**的——三份只有变体名与文案不同，且编号各从 1 起（与装配单那套号
//! 两套在跑）。故并成 [`super::Start`] 一枚（照实记见 `server.rs` 那一节）。

pub mod bridge;
pub mod server;
