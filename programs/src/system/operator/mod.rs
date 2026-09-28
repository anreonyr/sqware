//! operator::实现侧 — **持树者**那一域：三侧各住哪一份。
//!
//! ```text
//!   核（结构 ＋ 它的方法，纯）     core/       Operator（树 ＋ 七条原语）/ Ledger（一格两轴）
//!                                              judge（判据）/ gate（裁决）
//!   载体（本域表上那几枚孔）        claim.rs   按「谁开的 ＋ 记号」认领
//!   适配·装配侧（装配者手里那侧）   bridge.rs  Tree（接客人 / 递一条路）
//!   适配·持树侧（本域那一枚线程）   server.rs  收一句、交给谁
//!                                   plate.rs   提示之路·一条路 → 核 ＋ 账
//!                                   answer.rs  客人的一句问 → 核 ＋ 账
//!                                   door.rs    门外那一问 → Facts ＋ 裁决
//!   这一族的坐标与铸               mount.rs   SEGMENT ＋ entry(grant)
//! ```
//!
//! **一份一句话**：核只判只记（不认识 runtime / 协议 / 线程）；适配一份一族（把外面的话翻成核的
//! 话），**互不调用**；`server.rs` 是这一域的入口（那一枚线程）；`claim.rs` 在它们下面（只认本域
//! 那张表与记号）。
//!
//! 与 [`crate::system::board`] 同一个判据：**规范与接口**（正文、判定、帧、客侧三手、装配侧
//! `attach`/`host_of`）住 `crates/protocol/src/system/operator/`；**实现方**（**独立域**，
//! `prog-operator` 那一台 + 它记的账）住这里。
//!
//! **照实记（`core` 是残枝那一刀从 protocol 搬来的）**：树那本账、归属、门外那一问、
//! 裁决折线上一格，原住 `crates/protocol/src/system/operator/core/`——读者只有本域，故回这里。

pub mod bridge;
pub mod core;
pub mod mount;
pub mod server;

// 持树侧那三份 ＋ 本域表上那几枚孔的认领：只有本域这一份入口（`server`）叫它们。
mod answer;
mod claim;
mod door;
mod plate;
