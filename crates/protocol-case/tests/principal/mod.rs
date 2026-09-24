//! `crate::system::principal::core` 这个名字的**落点**。
//!
//! 盟籍那一份核心写的是 `use crate::system::principal::core::PrincipalId`——它要身份那本册子的号，
//! 而这台宿主靶里没有 `protocol` 的模块树。故这里照着它要的名字把它接上：一个真实的目录模块
//! `principal/`，里面是那份**逐字未改**的源码作为 `core`。
//!
//! **照实记**：一开始写的是**内联**模块（`mod principal { #[path = "…"] pub mod core; }`），
//! 编不过——`#[path]` 的基准会变成 `tests/principal/`，而那个目录在盘上是**虚的**（内联模块
//! 没有自己的文件），`..` 走不过去。改成真实目录模块之后基准是 `tests/principal/`（真的），
//! 三个 `..` 正好回到 `crates/`。

#[path = "../../../protocol/src/system/principal/core.rs"]
pub mod core;

/// **帧那一半**（`crates/protocol/src/system/principal/frame.rs`，逐字未改）—— 在本台里跑判据。
///
/// 它要 `env` 与同层 `core`（两个都在），外加那张 `fail_codes!` 表（宏自己一份源，见
/// `tests/roster.rs` 里那行 `#[macro_use]`）。适配那半（`call.rs`）拖 `session::call`，故不来。
#[allow(dead_code)]
#[path = "../../../protocol/src/system/principal/frame.rs"]
pub mod frame;
