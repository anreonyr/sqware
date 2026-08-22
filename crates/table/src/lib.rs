//! table — 无堆表格渲染，拆成四个子模块：
//!
//!   sym    —— 地址符号化（全局 set-once 注入）
//!   hex    —— 地址显示（render_addr / addr_width，渲染与宽度同源）
//!   fmt    —— 行缓冲格式化器（Fmt：拼一行 → 一次 flush，每格式一方法）
//!   table  —— 行导向有界格子表格，自研渲染（per-column 对齐；直写 sink）
//!
//! 全部锁死无分配（崩溃现场无堆无锁）：
//!   · 渲染只走 &mut fmt::Write，绝不 to_string()（std 分配出口）。
//!   · hex 的 render 与 addr_width 同源，表格列对齐才成立。

#![no_std]

mod fmt;
mod hex;
mod para;
mod sym;
mod table;

pub use fmt::Fmt;
pub use hex::{addr_width, render_addr};
pub use para::Para;
pub use sym::{SymFn, set_symbolizer};
pub use table::{Align, Cell, RowsMut, Table, Width};
