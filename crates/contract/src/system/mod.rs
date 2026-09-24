//! system — 编排协议**已搬进「约」的那几件**（形与据）。
//!
//! **照实记（中途两处正文）**：搬家分批 ⇒ 一份协议的**正文**与它的**件**暂时分居两个 crate。
//! 正文（那句话是什么、有哪些动作）留在 `protocol` 那一侧的 `system/mod.rs`——它要与还没搬的
//! `board::call` / `client` 一起看；最后一批搬完，正文挪到这儿，本文件接过它。
//! 这一份只做**声明**，不写第二遍正文。

pub mod board;
pub mod coalition;
pub mod core;
pub mod desk;
pub mod grant;
pub mod operator;
pub mod principal;
