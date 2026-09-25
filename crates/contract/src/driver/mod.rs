//! driver — **设备轴**里**已搬进「约」的那几件**（形与据）。
//!
//! **照实记（这一轴搬齐了）**：`supply` 与 `line` 两族都过来了——它们从前卡在同一处
//! （`crate::session::Pier` 的推/收要碰内核）。`Pier` 手表化（刀二 a）之后，"碰内核的那一手"
//! 全在表上，于是两族的形、据、客侧一起归位。正文（`driver/mod.rs` 那一份）仍留在 `protocol`，
//! 等最后一批搬。

pub mod line;
pub mod supply;
