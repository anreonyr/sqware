//! operator::实现侧 — **持树者**那一域：三侧各住哪一份。
//!
//! ```text
//!   核（结构 ＋ 它的方法，纯）     core/       Operator（树 ＋ 七条线上原语 ＋ 三条给判据的）
//!                                              judge（判据）/ gate（裁决）
//!   载体（本域表上那几枚孔）        claim.rs   按「谁开的 ＋ 记号」认领
//!   适配·装配侧（装配者手里那侧）   bridge.rs  Tree（接客人 / 递一条路）＋ land（各域自己落门牌那一趟）
//!   适配·持树侧（本域那一枚线程）   server.rs  收一句、交给谁
//!                                   plate.rs   提示之路·一条路 → 核
//!                                   answer.rs  客人的一句问 → 核
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
//! **照实记（`core` 是残枝那一刀从 protocol 搬来的）**：树、门外那一问、裁决折线上一格，原住
//! `crates/protocol/src/system/operator/core/`——读者只有本域，故回这里。**归属那一本账后来整本
//! 退场**：它是树的影子（`name` / `id` / 那一枚句柄都已经在树上），两轴如今都住在砖上。

use protocol::system::operator as ocall;

/// **那一段目录的名字**（`/svc/operator` 底下那一段，也即 `/svc/operator/{面名}` 的中间那一段）。
///
/// **它为什么住这里**（照实记：回炉那一刀把 `mount.rs` 整份收了）：那一段名字是**这一族自己的
/// 事实**，而"铸入口"那一手四族逐字同构、已收进 [`crate::system::mount::entry`]；一份文件只剩
/// 一条 `const` 就挣不来一个文件。名字的唯一来源在协议那一侧那一格（`ocall::NAME`），这里只引用。
pub const SEGMENT: &str = ocall::NAME;

pub mod bridge;
pub mod core;
pub mod server;

// 持树侧那三份 ＋ 本域表上那几枚孔的认领：只有本域这一份入口（`server`）叫它们。
mod answer;
mod claim;
mod door;
mod plate;
