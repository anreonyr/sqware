//! `crate::principal::core` 这个名字的**落点**。
//!
//! 盟籍那一份核心写的是 `use crate::principal::core::PrincipalId`——它要身份那本册子的号，
//! 而这台宿主靶里没有 `protocol` 的模块树。故这里照着它要的名字把它接上：一个真实的目录模块
//! `principal/`，里面是那份**逐字未改**的源码作为 `core`。
//!
//! **照实记**：一开始写的是**内联**模块（`mod principal { #[path = "…"] pub mod core; }`），
//! 编不过——`#[path]` 的基准会变成 `tests/principal/`，而那个目录在盘上是**虚的**（内联模块
//! 没有自己的文件），`..` 走不过去。改成真实目录模块之后基准是 `tests/principal/`（真的），
//! 三个 `..` 正好回到 `crates/`。

#[path = "../../../protocol/src/principal/core.rs"]
pub mod core;
