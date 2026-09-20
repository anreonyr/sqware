#![no_std]
//! programs — 镜像里装载的程序集合（**每个程序一份 `main.rs`**，就住在它那一片模块的目录里）。
//!
//! 本 lib 有两件共享物：`entry`（`_start` + panic 处理，各程序共用）与 [`supervisor`]
//! （**这台机器的装配面**：设备需求单、boot 给的账、按单起服务那一台机器）。
//!
//! **判据是「谁在说话」**：从外面找上某份协议的人用的一切（正文、判定、帧、**客侧那几手**）
//! 住 `crates/protocol`；那位协议的**实现方**（谁循环、谁记账、谁起线程、谁调内核）住本 crate
//! 的 `board/` `operator/` `system/` `firmware/` —— 也就是本 crate 的"干活"这一侧。
//! 共享的"干活"（装配机器、压测台）住 `supervisor/` 与 `stress/`：一份源码编一次，
//! 各 bin 只 `use`，不再有 `#[path]` 复制与"另一半是死码"的 `#[allow(dead_code)]`。
//!
//! **目录即程序**：每个程序的入口（`main.rs`）与它那一片模块同住一个目录——`system/` 里
//! 既有实现也有 `main.rs`；`root/` `plic/` `user/` `stress/` 各放自己的程序，`bin/` 那一层撤了。
//!
//! **设备侧同理**：谁要读设备，谁的目录里放自己的设备模块（`plic/` 下的
//! `plic.rs` / `uart.rs`）——**设备语义各带各的，装配契约才共享**。
//!
//! 其余驱动侧（名字→线号 / 终端渲染）随旧树一起清了（tag `proto-v1-baseline`），
//! 需要时按新形状写——**不从那一套搬**。
//!
//! 今天有**十七个**程序：`prog-echo` / `prog-guest` / `prog-passer` /
//! `prog-operator`（U 态）、`prog-root` / `prog-system` / `prog-plic`（监督者），加十台压测（`prog-churn` /
//! `prog-rig` / `prog-busy` / `prog-park` / `prog-hang` / `prog-load` / `prog-beat` /
//! `prog-again` / `prog-waiter` / `prog-group`）。**特权级不在这里声明**——那一格在
//! `kernel/build.rs::INITRD_BINS`。

extern crate alloc;

pub mod board;
pub mod entry;
pub mod firmware;
pub mod operator;
pub mod stress;
pub mod supervisor;
pub mod system;
