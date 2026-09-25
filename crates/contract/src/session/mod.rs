//! session — 会话协议**已搬进「约」的两件**：据（[`core`]）与手（[`hands`]）。
//!
//! **照实记（正文还在口那一侧）**：分批搬家的中途，正文（`session/mod.rs` 那一份"会话建立"
//! 的正文）留在 `protocol`，与还没搬的 `call` 一起看；本文件只做**声明**。
//!
//! **这一格是这一批最要紧的一处**：`core` 从前写着 `use super::call`——看着"不碰内核"，
//! 其实每一手都碰（靶里那份 224 行影子桩就是它的影子）。现在它只要一张 [`Hands`] 表。

pub mod core;
pub mod hands;

pub use core::{Claim, Pier, Quay, Seat};
pub use hands::{Hands, Hole};
