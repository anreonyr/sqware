#![no_std]
//! runtime — 镜像侧运行时：`no_std` 程序依赖内核的那一套。
//!
//! 两半分居两个模块（判据是"薄/厚"，不是行数）：
//!   `env`  —— envcall 转发（薄）：一次调用一个函数，零业务逻辑；
//!   `core` —— 组合与封装（厚）：把 `env` 的原语组装成 channel/unit/handshake 这类机制。
//!
//! 与 `crates/env` 的分工是**依赖方向**：`env` crate 是内核与用户态都要的 ABI
//! 线格式，本 crate 只跑在镜像侧。
//!
//! 边界：本 crate **不认识任何协议**（协议在 `crates/protocol`，反向依赖本 crate），
//! 也不认识任何程序（程序在 `programs`）。

extern crate alloc;

pub mod core;
pub mod env;

pub const PAGE_SIZE: usize = 4096;
