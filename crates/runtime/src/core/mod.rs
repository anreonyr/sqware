//! **三个 `core` 各不相同**，读到这个名字先看路径：
//!   `runtime::core`        本模块——本运行时自己的实现面；
//!   `kernel/src/runtime/`  内核给自己用的 switch/chrono/diagnose；
//!   `core::`（无前缀）      Rust 的 freestanding 核心库（`core::mem` 等）。
//!
//! 本 crate 只有两层：`core/` 是组合与封装（[`res`] 的四件厚壳、[`task`] 的任务本地原语、
//! [`exit`]），[`adapt`] 是唯一剩下的"调用方口径 → 内核口径"转换。
//! envcall 本身没有第二层：`crates/env` 生成的每格入口就是调用点直接叫的那一手。
//!
//! 例子：[`res::pie::HolePie`] 是门闩句柄（带期限循环），`res::port::Port` 是「厚」的一件——
//! **它厚在"配对"上，不厚在"往返"上**：多出来的只有"推的是哪一枚、收的是哪一枚、
//! 收的时候校来源"。编帧解帧、一问一答、开会话的握手都在 `crates/protocol`。

pub mod adapt;
pub mod exit;
pub mod res;
pub mod task;
