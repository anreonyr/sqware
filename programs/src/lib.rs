#![no_std]
//! 镜像里装载的程序集合：**每个程序一份 `main.rs`**，住它自己的目录里。
//!
//! 分档按特权级（[`system`] 是 S 态，[`user`] 是 U 态）；**两档都不算"特权"——只是"哪个域跑"**。

extern crate alloc;

pub mod boot;
pub mod driver;
pub mod entry;
pub mod harness;
pub mod service;
pub mod system;
pub mod unit;
pub mod user;

// 出口那一套的转发：生成物（`entry_<路径>.rs`）里写的是 `programs::…`，各 bin 的
// `main` 返回类型也写 `programs::Report` / `programs::Exit`，故这几个名字得在 crate 根上
pub use runtime::core::exit::{Exit, Report};

pub use env::Reason;

/// 入口那一手（过程宏）：bin 里写 `#[entry] fn main() …`。
pub use mold::entry;
