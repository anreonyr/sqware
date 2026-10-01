#![no_std]
//! programs — 镜像里装载的程序集合（**每个程序一份 `main.rs`**，就住在它那一片模块的目录里）。
//! **分档按特权级**（唯一声明处：`programs::unit::PROGRAMS` 里这一行的 `kind`）：[`root`] 与 [`system`]
//! 是 S 态那一档（引导域 / 编排域），[`user`] 是 U 态那一档（今天**只剩 `canonical`**：控制台那一台；

extern crate alloc;

pub mod driver;
pub mod entry;
pub mod root;
pub mod service;
pub mod system;
pub mod unit;
pub mod user;

// 出口那一套的转发：生成物（`entry_<路径>.rs`）里写的是 `programs::…`，各 bin 的
// `main` 返回类型也写 `programs::Report` / `programs::Exit`，故这几个名字得在 crate 根上
pub use runtime::core::exit::{Exit, Report};

pub use env::Reason;
/// 入口那一手（过程宏）：bin 里写 `#[entry] fn main() …`，展开与符号名见那个 crate。
pub use mold::entry;
