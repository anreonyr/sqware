#![no_std]
//! programs — 镜像里装载的程序集合（每个 `src/bin/` 一个）。
//!
//! 本 lib 只有一件共享物：`entry` —— `_start` + panic 处理，各程序共用。
//!
//! **装配面不回这里**：`needs`（要什么）、`pairing`（记录怎么编解码）住在 `bin/supervisor/`
//! 里，由两个 supervisor bin 各自 `#[path]` 声明（见 `needs.rs` 头注）；`service`（装配单）
//! **只由 `root` 声明**——它自己 `use super::board/needs/pairing`，回不到本 lib 来。这些都是
//! **这几个程序之间**的约定，不是全 crate 的公共面。
//!
//! **设备侧同理**：谁要读设备，谁的目录里放自己的设备模块（`bin/supervisor/plic/` 下的
//! `plic.rs` / `uart.rs`）——**设备语义各带各的，装配契约才共享**。
//!
//! 其余驱动侧（名字→线号 / 终端渲染）随旧树一起清了（tag `proto-v1-baseline`），
//! 需要时按新形状写——**不从那一套搬**。
//!
//! `bin/` 里今天是**十六个**：`prog-echo` / `prog-guest` / `prog-passer` /
//! `prog-operator`（U 态）、`prog-root` / `prog-plic`（监督者），加十台压测（`prog-churn` /
//! `prog-rig` / `prog-busy` / `prog-park` / `prog-hang` / `prog-load` / `prog-beat` /
//! `prog-again` / `prog-waiter` / `prog-group`）。**特权级不在这里声明**——那一格在
//! `kernel/build.rs::INITRD_BINS`。

pub mod entry;
