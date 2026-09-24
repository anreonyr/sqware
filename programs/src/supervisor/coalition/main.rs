#![no_std]
#![no_main]

//! coalition — **结盟服务**（U 态，独立域，**一枚线程**）。
//!
//! 本域只做一件事：起那本盟册（一条关系 + 一枚计数器），然后招待所有客人（[`serve`] 一枚
//! 线程招待到底）。装配（谁在什么时候 `derive` + `bind`）归编排域那一侧（`service.rs`）。
//!
//! # 为什么是 U 态
//!
//! 它**不持有、不授予、不解释任何 Pie**——只读写自己那两张格。故它不进"转授权中枢"那一档
//! （`operator` 是 S 态，因为它在树上 `ship` 带 `VEST` 的副本），与 [`principal`] / `echo` /
//! `guest` 同档（`env::assembly::ALL` 里这一行的 `kind`）。它要的几手（读 `Sire`、铸孔、开会话、挂组、
//! 按名找身份服务）都不需要 S 态——`rtc` 那一面量过同一条。
//!
//! **它与身份服务那一台同档，但多一个客人身份**：起手要在树上找到 `/sys/principal`，
//! 每条**写**原语嵌一次 `Resolve(发送者)`。那也是 U 态够用的（一问一答而已）。

extern crate programs;

// 本域只跑服务那一侧（`serve`）；客侧那一面住在 protocol 里，本域用不到。
use programs::supervisor::coalition::server;

/// 本 bin 的 `main`：服务那一侧跑完/起不来都把死法带回来——出口那一手由构建脚本生成
/// （见 `programs/build.rs`），本文件一个字都不碰它。
fn main() -> Result<(), programs::supervisor::coalition::fail::Fail> {
    server::serve()
}

// 本 bin 的入口那一手（`_start` 的汇编胶水 + 出口点）由构建脚本生成——见 `programs/build.rs`。
include!(concat!(env!("OUT_DIR"), "/entry_supervisor_coalition_main.rs"));
