//! principal 的**适配那一半** —— 内核那几只手的别名（帧与码见 [`frame`](super::frame)）。
//!
//! `pub use super::frame::*;` 把帧 / 码 / 记号那一片照旧转出来 ⇒ **调用点一处都不用改**
//! （`principal::call::RESOLVE`、`principal::call::BACK`、`principal::mod` 里那句 `pub use call::{…}`
//! 全都照旧）。结构那一格与用户裁定见 `docs/frame-gate.md`（**甲**：宏独立成一份源；
//! 帧那一半上宿主，先做 `line` / `principal` / `coalition` 三份）。

pub use super::frame::*;

pub use crate::session::call::opened_by;
