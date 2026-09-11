#![no_std]
//! protocol — 用户态协议层（**内核不依赖本 crate**）。
//!
//! 三方分工按**依赖方向**，不按行数：
//!   `env`      = ABI：内核与用户态**都要**的线格式与调用骨架（`EnvCall`/`wire`/`Permission`）；
//!   `runtime`  = 机制：把 ABI 落成可用的运行时（`env` 薄转发 + `core` 组合封装）；
//!   `protocol` = 语义：只跑在用户态、且**只对某个协议有意义**的东西。
//!
//! 依赖：`protocol → runtime → env`（单向，无环）。
//!
//! 为什么要有这一层：目录协议原先住在 `crates/env` 里，于是"内核也依赖的 ABI crate"
//! 里躺着 284 行内核永远读不到的协议（全仓内核引用数 = 0，实测），"这是用户态的东西"
//! 这句话从结构上被抹掉了。独立成 crate 之后，**内核不知道目录协议**由编译期依赖
//! 保证：`kernel/Cargo.toml` 里没有 `protocol`，想引用也引用不到。
//!
//! 为什么现在是 `→ runtime` 而不是 `→ env`：协议的用户态实现要用**机制**
//! （门闩 `HolePie`、回信通道 `Channel`），而机制在 `runtime`。这一条边就是
//! "机制在运行时、语义在协议"这句话的编译期形态。

extern crate alloc;

pub mod console;
pub mod dispatch;
pub mod doom;
pub mod irq;
