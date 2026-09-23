//! `session` 这个名字的落点：**桩 + 核心**。
//!
//! 核心那份源码里写着 `use super::call;` ⇒ 它要的 `call` 与它自己同住 `session` 这一层。
//! 故这里是一个**真实的目录模块**（`tests/session/`），里面两个文件：桩 `call.rs`（照
//! `crates/line-case` 给 `Pier` 打桩的同一条路数）与**逐字未改**的 `core.rs`。
//!
//! 照实记：上一轮 `principal-case` 里先写的**内联**模块编不过——内联模块没有自己的文件，
//! `#[path]` 的基准目录在盘上是虚的，`..` 走不过去。故这一台一步到位用真实目录。

/// 运行时那一层的**桩**：会话核心只跟它说那几句话（见该文件）。
pub mod call;

/// 会话核心（就是 `crates/protocol/src/session/core.rs` 那一份，逐字未改）。
#[path = "../../../protocol/src/session/core.rs"]
pub mod core;
