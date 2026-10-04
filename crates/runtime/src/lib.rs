#![no_std]
//! runtime — 镜像侧运行时：`no_std` 程序依赖内核的那一套。
//!
//! 一层：`core` —— 组合与封装。四种资源各一件厚壳（Hole → [`core::res::port`]、
//! Pole → [`core::res::dock`]、Nole → [`core::res::bell`]、Tole → [`core::res::pile`]）
//! 持着 [`core::res::pie`] 的四枚薄句柄；加任务本地原语（`heap` / `lock` / `tls` /
//! `unit`）与唯一的调用口径转换 [`core::adapt`]。
//!
//! **envcall 没有转发层**：`crates/env` 生成的每格入口（`env::pie::seal(token)`、
//! `env::room::starve()`…）就是调用点直接叫的那一手——`runtime` 只补真正的转换
//! （`&[T]` → `(ptr, len)`、`usize` ↔ `VirtAddr`、`Duration` → 毫秒向上取整…），
//! 不为每一格再立一个同名函数。
//!
//! 与 `crates/env` 的分工是**依赖方向**：`env` crate 是内核与用户态都要的 ABI 线格式，
//! 本 crate 只跑在镜像侧。
//!
//! 边界：本 crate **不认识任何协议**（协议在 `crates/protocol`，反向依赖本 crate），
//! 也不认识任何程序（程序在 `programs`）。

extern crate alloc;

pub mod core;

pub const PAGE_SIZE: usize = 4096;
