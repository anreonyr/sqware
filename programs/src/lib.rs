#![no_std]
//! 镜像里装载的程序集合：**每个程序一份 `main.rs`**，住它自己的目录里。
//!
//! 分档按特权级：S 态 composition entry 仍由本仓 `system` bin 持有，
//! 运行时 body 由外部 `system-server` 提供；[`user`] 是 U 态。
//! **两档都不算"特权"——只是"哪个域跑"**。

extern crate alloc;
extern crate self as programs;

#[doc(hidden)]
pub use alloc::format as __format;
pub use system_support::{boot, debug, support};

pub mod driver;
pub mod harness;
mod interfaces;
pub mod service;
pub mod user;

pub use composition;
pub use unit;

// 出口那一套的转发：生成物（`entry_<路径>.rs`）里写的是 `programs::…`，各 bin 的
// `main` 返回类型也写 `programs::Report` / `programs::Exit`，故这几个名字得在 crate 根上
pub use execution::boot::{Exit, Report};

pub use env::Reason;

/// 入口那一手（过程宏）：bin 里写 `#[entry] fn main() …`。
pub use mold::entry;
