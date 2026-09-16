#![no_std]
//! programs — 镜像里装载的程序集合（每个 `src/bin/` 一个）。
//!
//! 本 lib 只有一件共享物：`entry` —— `_start` + panic 处理，各程序共用。
//!
//! **装配面不回这里**：`needs`（要什么）、`pairing`（记录怎么编解码）、`service`（装配单）
//! 住在 `bin/supervisor/` 里，由两个 supervisor bin 各自 `#[path]` 声明（见 `needs.rs`
//! 头注）——它们是**这几个程序之间**的约定，不是全 crate 的公共面。
//!
//! **设备侧同理**：谁要读设备，谁的目录里放自己的设备模块（`bin/supervisor/plic/` 下的
//! `plic.rs` / `uart.rs`）——**设备语义各带各的，装配契约才共享**。
//!
//! 其余驱动侧（名字→线号 / 终端渲染）随旧树一起清了（tag `proto-v1-baseline`），
//! 需要时按新形状写——**不从那一套搬**。
//!
//! `bin/` 里有三个：`prog-root`（装配者）、`prog-plic`（中断面域）、`prog-echo`（调试回显）。

pub mod entry;
