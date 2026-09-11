//! core — 运行时的内核实现面：把 `env` 的原语组装成可用的机制。
//! 任务本地原语（heap/tls/unit）+ Mail 之上封装（channel/handshake）。
//! **协议不在本 crate**：目录协议的客户端与服务端已搬去 `crates/protocol::dispatch`，
//! 本模块对 `protocol` 零引用。
//!
//! **三个 `core` 各不相同**，读到这个名字先看路径：
//!   `runtime::core`        本模块——本运行时自己的实现面；
//!   `kernel/src/runtime/`  内核给自己用的 switch/chrono/diagnose；
//!   `core::`（无前缀）      Rust 的 freestanding 核心库（`core::mem` 等）。
//!
//! 与 `env/` 的分工：`env/` 是 envcall 转发（薄），`core/` 是组合与封装（厚）。
//! 例子：`env::mail::HolePie` 是「薄」门闩句柄；`core::channel::Channel` 是
//! 「厚」开-关生命周期封装。

pub mod channel;
pub mod handshake;
pub mod heap;
pub mod lock;
pub mod tls;
pub mod unit;
