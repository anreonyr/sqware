//! 盟籍那两片：**账**（`core.rs`）与**帧**（`frame.rs`），都在这一层。
//!
//! 用**真实目录模块**（理由与踩过的那个坑见 `tests/principal/mod.rs`）：帧那一份写的是
//! `use super::core::{…}`，故两片必须同住一层、名字照旧。

/// 盟册的正文（就是 `crates/protocol/src/system/coalition/core.rs` 那一份，逐字未改）。
#[path = "../../../protocol/src/system/coalition/core.rs"]
pub mod core;

/// **帧那一半**（`crates/protocol/src/system/coalition/frame.rs`，逐字未改）—— 在本台里跑判据。
#[allow(dead_code)]
#[path = "../../../protocol/src/system/coalition/frame.rs"]
pub mod frame;
