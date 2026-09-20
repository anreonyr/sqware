#![no_std]
//! programs — 镜像里装载的程序集合（每个 `src/bin/` 一个）。
//!
//! 本 lib 有两件共享物：`entry`（`_start` + panic 处理，各程序共用）与 [`supervisor`]
//! （**这台机器的装配面**：设备需求单、boot 给的账、按单起服务那一台机器）。
//!
//! **共享面住 lib，接口面住 `crates/protocol`**：一份源码只编一次，各 bin 只 `use`，
//! 不再有 `#[path]` 复制与"另一半是死码"的 `#[allow(dead_code)]`。协议面（判定 / 帧 / 账 /
//! 三个角色：`server` / `client` / `bridge`）住在 `crates/protocol` 的每一份正文旁边。
//!
//! **设备侧同理**：谁要读设备，谁的目录里放自己的设备模块（`bin/supervisor/plic/` 下的
//! `plic.rs` / `uart.rs`）——**设备语义各带各的，装配契约才共享**。
//!
//! 其余驱动侧（名字→线号 / 终端渲染）随旧树一起清了（tag `proto-v1-baseline`），
//! 需要时按新形状写——**不从那一套搬**。
//!
//! `bin/` 里今天是**十七个**：`prog-echo` / `prog-guest` / `prog-passer` /
//! `prog-operator`（U 态）、`prog-root` / `prog-system` / `prog-plic`（监督者），加十台压测（`prog-churn` /
//! `prog-rig` / `prog-busy` / `prog-park` / `prog-hang` / `prog-load` / `prog-beat` /
//! `prog-again` / `prog-waiter` / `prog-group`）。**特权级不在这里声明**——那一格在
//! `kernel/build.rs::INITRD_BINS`。

extern crate alloc;

pub mod entry;
pub mod stress;
pub mod supervisor;
