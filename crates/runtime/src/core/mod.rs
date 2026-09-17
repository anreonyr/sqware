//! core — 运行时的内核实现面：把 `env` 的原语组装成可用的机制。
//! 任务本地原语（heap/tls/unit）+ Mail 之上封装（port/dock/bell）——**三件各包一种
//! primitive**：Hole → `Port`、Pole → `Dock`、Nole → `Bell`。
//! **协议不在本 crate**：本模块对 `protocol` 零引用。
//!
//! **三个 `core` 各不相同**，读到这个名字先看路径：
//!   `runtime::core`        本模块——本运行时自己的实现面；
//!   `kernel/src/runtime/`  内核给自己用的 switch/chrono/diagnose；
//!   `core::`（无前缀）      Rust 的 freestanding 核心库（`core::mem` 等）。
//!
//! 与 `env/` 的分工：`env/` 是 envcall 转发（薄），`core/` 是组合与封装（厚）。
//! 例子：`env::mail::HolePie` 是「薄」门闩句柄；`core::port::Port` 是「厚」的一件——
//! **但它厚在"配对"上，不厚在"往返"上**：`open` / `push` / `pull` / `shut` 与 `HolePie`
//! 那一族同名同形，多出来的只有"推的是哪一枚、收的是哪一枚、收的时候校来源"。
//! 编帧解帧、一问一答、开会话的握手都在 `crates/protocol`（它们的消费者在那里）。

pub mod bell;
pub mod dock;
pub mod heap;
pub mod lock;
pub mod port;
pub mod tls;
pub mod tole;
pub mod unit;
