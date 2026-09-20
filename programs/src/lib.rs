#![no_std]
//! programs — 镜像里装载的程序集合（**每个程序一份 `main.rs`**，就住在它那一片模块的目录里）。
//!
//! **分档按特权级**（唯一声明处：`kernel/build.rs::INITRD_BINS`）：[`supervisor`] 是 S 态那一档
//! （root / plic / system），[`user`] 是 U 态那一档（operator / echo / guest / passer）；
//! 压测台**不分档**，整块留在 [`stress`]。
//!
//! 本 lib 另有一件共享物：`entry`（`_start` + panic 处理，各程序共用）。
//!
//! **判据是「谁在说话」**：从外面找上某份协议的人用的一切（正文、判定、帧、**客侧那几手**）
//! 住 `crates/protocol`；那位协议的**实现方**（谁循环、谁记账、谁起线程、谁调内核）跟着
//! **用它那个程序所在的档**走——`supervisor/{board,firmware,system}` 与 `user/operator`。
//! 共享的"干活"住 `supervisor/` 本级与 `stress/`：一份源码编一次，各程序只 `use`，不再有
//! `#[path]` 复制与"另一半是死码"的 `#[allow(dead_code)]`。
//!
//! **目录即程序**：每个程序的入口（`main.rs`）与它那一片模块同住一个目录——`supervisor/system/`
//! 里既有实现也有 `main.rs`，`supervisor/root/`、`supervisor/plic/`、`user/operator/` 同理；
//! `bin/` 那一层撤了。压测的十份入口与共用的 `tick` 一起住 `stress/`。
//!
//! **设备侧同理**：谁要读设备，谁的目录里放自己的设备模块（`supervisor/plic/` 下的
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

pub mod entry;
pub mod stress;
pub mod supervisor;
pub mod user;
