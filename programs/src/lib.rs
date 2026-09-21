#![no_std]
//! programs — 镜像里装载的程序集合（**每个程序一份 `main.rs`**，就住在它那一片模块的目录里）。
//!
//! **分档按特权级**（唯一声明处：`kernel/build.rs::INITRD_BINS`）：[`supervisor`] 是 S 态那一档
//! （root / router / uart / system / operator），[`user`] 是 U 态那一档（echo / guest / passer）。
//! **按角色分的两族不分档**：压测台整块留在 [`stress`]，**驱动整块留在 [`driver`]**——成员的
//! 特权级仍各自在 `INITRD_BINS` 声明（今天两台驱动都是 S 态；见 [`driver`] 的头注）。
//!
//! **表归主人**：硬件需求单在**收方**（`driver/router/needs.rs`、`driver/uart/needs.rs`：本域要
//! 哪几枚、落到它自己那张表的第几格）；boot 的两块账在**引导域**（`supervisor/root/boot.rs`：
//! 只有它读得到）——装配者只是 `use` 它们，不另抄一份。
//!
//! 内含之后**共用件只剩三枚**：`entry`（`_start` + panic 处理，每个程序共用）、
//! [`supervisor::service`]（那台装配机器，两个装配者 `root` / `system` 共用）与
//! [`driver::assemble`]（**客侧**那台机器，两台驱动共用）。
//!
//! **判据是「谁在说话」**：从外面找上某份协议的人用的一切（正文、判定、帧、**客侧那几手**）
//! 住 `crates/protocol`；那位协议的**实现方**（谁循环、谁记账、谁起线程、谁调内核）跟着
//! **用它那个程序所在的档**走——`supervisor/{supply,operator,system}`（板的实现方就在
//! `supervisor/system/board/` 之下：板线程是编排域里的一枚线程，不是另一个域）。
//! 共享的"干活"住 `supervisor/` 本级、`driver/` 本级与 `stress/`：一份源码编一次，各程序只
//! `use`，不再有 `#[path]` 复制与"另一半是死码"的 `#[allow(dead_code)]`。
//!
//! **目录即程序**：每个程序的入口（`main.rs`）与它那一片模块同住一个目录——`supervisor/system/`
//! 里既有实现也有 `main.rs`，`supervisor/root/`、`supervisor/operator/`、`driver/router/`、
//! `driver/uart/` 同理；`bin/` 那一层撤了。压测的十份入口与共用的 `tick` 一起住 `stress/`。
//!
//! **设备侧同理**：谁要读设备，谁的目录里放自己的设备模块（`driver/router/` 下的 `plic.rs`、
//! `driver/uart/` 下的 `uart.rs`）——**设备语义各带各的，装配契约才共享**。
//!
//! 其余驱动侧（名字→线号 / 终端渲染）随旧树一起清了（tag `proto-v1-baseline`），
//! 需要时按新形状写——**不从那一套搬**。
//!
//! 今天有**十八个**程序：`prog-echo` / `prog-guest` / `prog-passer`（U 态）、
//! `prog-root` / `prog-system` / `prog-router` / `prog-uart` / `prog-operator`（监督者），
//! 加十台压测（`prog-churn` / `prog-rig` / `prog-busy` / `prog-park` / `prog-hang` /
//! `prog-load` / `prog-beat` / `prog-again` / `prog-waiter` / `prog-group`）。
//! **特权级不在这里声明**——那一格在 `kernel/build.rs::INITRD_BINS`。

extern crate alloc;

pub mod driver;
pub mod entry;
pub mod stress;
pub mod supervisor;
pub mod user;
