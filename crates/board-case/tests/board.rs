//! 公示板与待客台账的门（**宿主台**）—— 板核心的规矩，在宿主上真跑一遍。
//!
//! # 这一台为什么存在（照实记：这一批是"救活的"）
//!
//! `crates/protocol/src/system/board/core.rs` 的 `#[cfg(test)]` 模块**从写下那天起一次没跑过**
//! ——`protocol` 是 `[lib] test = false`（riscv 上编不出 libtest），主工作区那几道门一道都不编它。
//! 与别的几台同一条路（**那张清单与理由住 `scripts/host.sh` 的头注**——这里不写台数：
//! 台数每加一台就要改一遍，而"话要能指回源头"）：编外宿主 crate、只依赖 `env`、
//! 把核心源码**逐字未改**地 `#[path]` 进来，
//! 门口 `scripts/host.sh`。这一份**无桩**（只认 `env` 那几个类型）。
//!
//! # 这一台钉的是什么
//!
//! 见那一份源码自己的 `#[cfg(test)]` 模块（板那一格的规矩：立牌子、摘牌子、查名字、退场）。

extern crate alloc;

/// 板那本账（就是 `crates/protocol/src/system/board/core.rs` 那一份，逐字未改）。
#[path = "../../protocol/src/system/board/core.rs"]
mod board;
