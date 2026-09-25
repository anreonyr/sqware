//! `system` 这个名字的落点：编排那一层的三份核心（账 / 判定 / 配给）。
//!
//! **真依赖** `contract` 的那三份——`core.rs` 里那句 `use super::desk::{…}` 要的 `desk`
//! 正是同一个 `system` 之下的兄弟模块，在那边本来就成立。
//!
//! **照实记（`#[path]` 退场）**：这三份原先各拿一行 `#[path]` 逐字编进靶，`desk` / `core`
//! 上还各挂一个 `#[allow(dead_code)]`（本台**只叫其中一部分**——比如 `core.rs` 的 `Reaped`
//! 与 `Fail::BadImage` 由适配层用，本台叫不到；当年挂 allow 而不是删那几条，是因为那是逐字
//! 未改的源码）。转出真那份之后那个 allow 也多余：`dead_code` 按**定义它的 crate** 算，
//! `contract` 是依赖、不重算。故三行 `#[path]` ＋ 两个 allow 一起退场，判据的对象仍是同一份源码。
//!
//! **照实记（为什么从前得用真实目录模块）**：内联模块的 `#[path]` 基准目录在盘上是虚的，
//! 走不过去（见 `roster` 靶）。那一格随 `#[path]` 一起消失。

/// 账 / 判定 / 配给——**真依赖** `contract::system` 那三份（逐字同一份源码）。
pub use contract::system::{core, desk, grant};
