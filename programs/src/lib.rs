#![no_std]
//! programs — 镜像里装载的程序集合（每个 `src/bin/` 一个）。
//!
//! 本 lib 收程序侧共享物：`entry`（`_start`/panic 处理，各程序共用）、两台设备的
//! **设备侧** `uart` 与 `plic`（同形：`open` + 职业动作，**不是设备框架**，见各自模块头）
//! 与 `lines`（设备树 → 行：名字 → 线号，见 `docs/driver.md` §12；表的语义在
//! `protocol::irq::server`）。
//! `bin/` 是**同时依赖 runtime 与 protocol 的装配层**：机制来自 `runtime`，
//! 协议语义来自 `protocol`。
//!
//! **`term` 已不在本包**：终端渲染与行编辑搬进 `prog-console` 服务，
//! 住在 `crates/protocol/src/console/server.rs`；程序侧只剩线对侧。
//!
//! **线表也不在本包**：`lines` 只留"设备树怎么收成行"（驱动的设备知识），而
//! 属主 / 委托 / 实例 / 收线那几条判据搬进 `crates/protocol/src/irq/server.rs`。

extern crate alloc;

pub mod entry;
pub mod lines;
// 两台设备的设备侧（同形：`open` + 职业动作，无协议、无循环、无配给）。
// 中断面的两个角色也各归各的：UART 是**源**（`drain`/`IER`），PLIC 是**控制器**
// （`claim`/`complete`/`enable`/`disable`）；把它们连起来的只有 `protocol::irq`。
pub mod plic;
pub mod uart;
