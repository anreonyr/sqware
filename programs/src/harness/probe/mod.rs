//! 探针那一档的各台（身子在各自的 `probe_<名>/main.rs`）与它们**共用的量具**。
//!
//! 各台仍是独立 bin：由 `programs/Cargo.toml` 的 `[[bin]]` 声明、由
//! `programs::unit::catalog` 的 `PROGRAMS` 表登记它那一份 `UnitFile`。共用的东西住**本目录顶层**：
//! - [`count`]：**数一块窗格底下到齐没有**（各族"该有几枚"由各自的 `Grant::ALL.len()` 说）；
//! - [`rack`]：架那一族三台的**试验场**（那条路、那两枚砖、那一条载荷）。

pub mod count;
pub mod identity;
pub mod rack;

pub mod hierarchy;
