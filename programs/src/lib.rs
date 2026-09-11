#![no_std]
//! programs — 镜像里装载的程序集合（每个 `src/bin/` 一个）。
//!
//! 本 lib 收程序侧共享物：`entry`（`_start`/panic 处理，各程序共用）与
//! `uart`（一台设备的驱动——**不是设备框架**，见其模块头）。
//! `bin/` 是**同时依赖 runtime 与 protocol 的装配层**：机制来自 `runtime`，
//! 协议语义来自 `protocol`。
//!
//! **`term` 已不在本包**：终端渲染与行编辑搬进 `prog-console` 服务，
//! 住在 `crates/protocol/src/console/server.rs`；程序侧只剩线对侧。

extern crate alloc;

pub mod entry;
pub mod uart;
