#![no_std]
#![no_main]

//! operator — **那棵命名树的服务**（U 态，独立域，**一枚线程**）。
//!
//! 本域只做一件事：起那棵树，然后**招待所有客人**（[`serve`] 一枚线程招待到底）。装配
//! （谁跟它接上、提示孔怎么认）归 `root` 那一侧（`service.rs` 的 `Program::operator`）。
//!
//! # 为什么是 U 态
//!
//! 它只搬 Pie（`Accord` 的副本）——不碰 MMIO、不读设备、不建域。最小特权够用，故照
//! `echo` / `guest` 那一档报进 initrd（`kernel/build.rs::INITRD_BINS`）。
//!
//! # 为什么一枚线程
//!
//! 树上那几枚句柄是"**我这张表里的第几个**"：查到了要授出去，必须由**持有它的那张表**来做。
//! 故所有条目只能住同一张表，也就是同一枚线程——`supervisor/operator.rs` 的
//! "为什么持树者就一枚线程"写了这条（板那一台栽过一次）。

extern crate programs;

#[path = "../operator.rs"]
// 本域只用持树者侧那一半（`serve`）；装配侧与客侧那几手在这里是死码。
#[allow(dead_code)]
mod operator;

#[unsafe(no_mangle)]
extern "C" fn main() -> ! {
    operator::serve()
}
