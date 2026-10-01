#![no_std]
//! programs — 镜像里装载的程序集合（**每个程序一份 `main.rs`**，就住在它那一片模块的目录里）。
//! ```text
//!   driver/ service/ system/ user/   四档程序族（每族一台一份主模块，声明写在各台 `program.rs` 里）
//!   harness/  测具那一档（探针 / 试客 / 压测台；不进产品镜像）
//!   unit/     声明层：一台程序的模型（`UnitFile`）＋ 注册表 ＋ 次序 ＋ 测具那 25 台的声明
//!   boot/     每个引导镜像共用：两块账（accounts）与清单读面（catalog）
//!   entry.rs        每个程序共用：那一手 `_start`（`crates/mold` 的 `#[entry]` 写死这条路径）
//! ```
//! **分档按特权级**（唯一声明处：`programs::unit::PROGRAMS` 里这一行的 `kind`）：[`system`]
//! 是 S 态那一档（编排域——它也是引导镜像），[`user`] 是 U 态那一档（今天**只剩 `canonical`**：
//! 控制台那一台）。

extern crate alloc;

/// boot 交给**引导镜像那一域**的两块账与清单读面（每个引导镜像都读得到）。
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
/// 入口那一手（过程宏）：bin 里写 `#[entry] fn main() …`，展开与符号名见那个 crate。
pub use mold::entry;
