#![no_std]
#![no_main]

//! operator — **那棵命名树的服务**（S 态，独立域，**一枚线程**）。
//!
//! 本域只做一件事：起那棵树，然后**招待所有客人**（[`serve`] 一枚线程招待到底）。装配
//! （谁跟它接上、提示孔怎么认）归 `root` 那一侧（`service.rs` 的 `Program::operator`）。
//!
//! # 为什么是 S 态
//!
//! 它不建域、不碰 MMIO、不读设备——但**它是这台机器的转授权中枢**：谁在树上查到一条，它就
//! `ship` 一枚带 `VEST` 的副本出去（`protocol::operator::call::ship`）。故不进"最小特权"
//! 那一档（`echo` / `guest` / `passer` / `lodger`），与监督侧同档（`env::assembly::ALL` 里这一行的 `kind`）。
//!
//! # 为什么一枚线程
//!
//! 树上那几枚句柄是"**我这张表里的第几个**"：查到了要授出去，必须由**持有它的那张表**来做。
//! 故所有条目只能住同一张表，也就是同一枚线程——`protocol::operator::server` 的
//! "为什么持树者就一枚线程"写了这条（板那一台栽过一次）。

extern crate programs;

// 本域只跑持树者那一侧（`serve`）；装配侧与客侧住在 protocol 里，本域用不到。
use programs::supervisor::operator::server as operator;

/// 本 bin 的 `main`：服务那一侧跑完/起不来都把原因码带回来——出口那一手
/// （`_start` 胶水与 `Reap`）全在 [`programs::entry`]，本文件一个字都不碰它。
extern "C" fn bare_main() -> programs::Reason {
    operator::serve()
}

programs::boot!(bare_main);
