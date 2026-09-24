//! principal 的**适配那一半** —— 内核那几只手的别名（帧与码见 [`frame`](super::frame)）。
//!
//! `pub use super::frame::*;` 把帧 / 码 / 记号那一片照旧转出来 ⇒ **调用点一处都不用改**
//! （`principal::call::RESOLVE`、`principal::call::BACK`、`principal::mod` 里那句 `pub use call::{…}`
//! 全都照旧）。结构那一格：帧那一半拆去 [`frame`](super::frame)、上宿主靶跑判据（它只认
//! `env` 与同层 `core`），故本文件只剩内核那几只手的别名。

pub use super::frame::*;

pub use crate::session::call::opened_by;
