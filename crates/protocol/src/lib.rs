#![no_std]
//! protocol — 用户态协议层（**内核不依赖本 crate**）。
//!
//! 与 `crates/env` 的分工是**依赖方向**，不是行数：
//!   `env`      = ABI：内核与用户态**都要**的线格式与调用骨架（`EnvCall`/`wire`/`Permission`）；
//!   `protocol` = 协议：只跑在用户态的东西（服务目录的 `Request`/`Reply`/`MSG_LEN`）。
//!
//! 为什么要有这一层：目录协议原本住在 `crates/env` 里，于是"内核也依赖的 ABI crate"
//! 里躺着 284 行内核永远读不到的协议（全仓内核引用数 = 0，实测），"这是用户态的东西"
//! 这句话从结构上被抹掉了。独立成 crate 之后，**内核不知道目录协议**由编译期依赖
//! 保证：`kernel/Cargo.toml` 里没有 `protocol`，想引用也引用不到。
//!
//! 依赖：`protocol → env`（单向）。`task → protocol`。

pub mod dispatch;
