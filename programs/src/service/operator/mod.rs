//! operator::实现侧 — **持树者**那一域：三侧各住哪一份。
//! ```text
//!   核（结构 ＋ 它的方法，纯）     core/       Operator
//!                                              judge/ gate（裁决）
//!   载体（本域表上那几枚孔）        claim.rs   按「谁开的 ＋ 记号」认领
//!   适配·装配侧（装配者手里那侧）   bridge.rs  Tree（接客人 / 递一条路）＋ land（各域自己落门牌那一趟）
//!   适配·持树侧（本域那一枚线程）   serve/     收一句、交给谁
//!                                   plate.rs   提示之路·一条路 → 核
//!                                   answer.rs  客人的一句问 → 核
//!                                   door.rs    门外那一问 → Facts ＋ 裁决
//!   这一族的坐标与铸               mount.rs   entry(grant)（路 = 协议侧那枚 `DIR`）
//! ```
//! **一份一句话**：核只判只记（不认识 runtime / 协议 / 线程）；适配一份一族（把外面的话翻成核的
//! 话），**互不调用**；`serve/` 是这一域的入口（那一枚线程）；`claim.rs` 在它们下面（只认本域
//! 那张表与记号）。

pub mod bridge;
pub mod core;

// 持树侧那一侧（本域的一枚线程 ＋ 它叫的那三手）住在 `serve/`；载体 `claim.rs`
// 在它们下面（只认本域那张表与记号）。
mod claim;
/// 那一枚线程（服务侧 / 持树侧）与它叫的那几手住这里。
pub mod serve;
