//! table — 无堆表格渲染，拆成四个子模块：
//!
//!   sym    —— 地址符号化（全局 set-once 注入）
//!   hex    —— 地址显示（render_addr / addr_width，渲染与宽度同源）
//!   fmt    —— 行缓冲格式化器（Fmt：拼一行 → 一次 flush，每格式一方法）
//!   table  —— 有界格子表格，交给 papergrid 渲染
//!
//! 全部锁死无分配（崩溃现场无堆无锁）：
//!   · 渲染只走 build(&mut fmt::Write)（Display 路径），绝不 to_string()。
//!   · hex 的 render 与 addr_width 同源，表格列对齐才成立。

#![no_std]

mod fmt;
mod hex;
mod sym;
mod table;

pub use fmt::Fmt;
pub use hex::{addr_width, render_addr};
pub use sym::{set_symbolizer, SymFn};
pub use table::{Align, Cell, Line, Table};
