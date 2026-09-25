//! supply — **物料到手**这一族**全搬进「约」了**：据（`core`）、形（`frame`）、客侧（`client`）。
//!
//! **照实记（它从前卡在哪）**：客侧吃一枚 `&Pier`，而 `Pier` 的推/收要碰内核 ⇒ 只能等
//! `Pier` 手表化（刀二 a）。如今它拿的还是同一枚 `&Pier`，手却已经全在表上。
//! 正文见 `protocol` 那一侧的 `driver/supply/mod.rs`。

pub mod client;
pub mod core;
pub mod frame;
