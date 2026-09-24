#![no_std]
#![no_main]

//! principal — **身份服务**（U 态，独立域，**一枚线程**）。
//!
//! 本域只做一件事：起那份账（名册 + 谱系），然后招待所有客人（[`serve`] 一枚线程招待到底）。
//! 装配（谁在什么时候 `derive` + `bind`）归编排域那一侧（`service.rs`）。
//!
//! # 为什么是 U 态
//!
//! 它**不持有、不授予、不解释任何 Pie**——只读写自己那两张表。故它不进"转授权中枢"那一档
//! （`operator` 是 S 态，因为它在树上 `ship` 带 `VEST` 的副本），与 `echo` / `guest` 同档
//! （`env::assembly::ALL` 里这一行的 `kind`）。它要的几手（读 `Sire`、铸孔、开会话、挂组）都不需要
//! S 态——`rtc` 那一面量过同一条。

extern crate programs;

// 本域只跑服务那一侧（`serve`）；客侧那一面住在 protocol 里，本域用不到。
use programs::supervisor::principal::server;

/// 本 bin 的 `main`：服务那一侧跑完/起不来都把死法带回来——出口那一手由构建脚本生成
/// （见 `programs/build.rs`），本文件一个字都不碰它。
#[programs::entry]
fn main() -> Result<(), programs::supervisor::principal::fail::Fail> {
    server::serve()
}

