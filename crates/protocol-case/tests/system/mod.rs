//! `system` 这个名字的落点：编排那一层的三份核心（账 / 判定 / 配给）。
//!
//! 三个文件都是**逐字未改**的协议源码；`core.rs` 里那句 `use super::desk::{…}` 要的
//! `desk` 正是这里的兄弟模块。用**真实目录模块**（`tests/system/`）——内联模块的 `#[path]`
//! 基准目录在盘上是虚的，走不过去（照实记见 `protocol-case` 的 `roster` 靶）。

// 照实记：`#[allow(dead_code)]` 挂在这两份上，是因为本台**只叫了其中一部分**（比如
// `core.rs` 的 `Reaped` 与 `Fail::BadImage` 由适配层用，本台叫不到）。挂 allow 而不是把
// 那几条删掉：那是**逐字未改的源码**，改它就是改判据的对象（与 `protocol-case` 的 `judge` 靶同一条）。

/// 账：一张定长表（名字 / 身子 / 生命阶段 / 就绪凭据）。
#[allow(dead_code)]
#[path = "../../../contract/src/system/desk.rs"]
pub mod desk;

/// 判定：纯函数，只读表。
#[allow(dead_code)]
#[path = "../../../contract/src/system/core.rs"]
pub mod core;

/// 配给那一半：那段记录按步长解出来。
#[path = "../../../contract/src/system/grant.rs"]
pub mod grant;
