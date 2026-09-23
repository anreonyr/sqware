//! 名册与谱系（+ 盟籍）的门（**宿主台**）—— 三本册子的规矩，在宿主上真跑一遍。
//!
//! # 这一台为什么存在（照实记：这一批是"救活的"）
//!
//! `crates/protocol/src/principal/core.rs` 与 `crates/protocol/src/coalition/core.rs` 各自的
//! `#[cfg(test)]` 模块**从写下那天起一次没跑过**：`protocol` 是 `[lib] test = false`
//! （riscv 目标上编不出 libtest），而主工作区那几道门（`check --all-targets` /
//! `build --release`）一道都不编它——那批规格长期只有"写着的规格"、没有"跑着的判据"。
//!
//! 与别的几台同一条路（清单见 `scripts/host.sh` 头注；头几台是 `operator-case` /
//! `line-case` / `judge-case`）：编外宿主
//! crate、只依赖 `env`、把核心源码**逐字未改**地 `#[path]` 进来，门口 `scripts/host.sh`。
//!
//! **两本册子同住一台**：盟籍核心写着 `use crate::principal::core::PrincipalId` —— 它要身份
//! 那本册子的号。分两台各编一遍的话，`principal/core.rs` 里那批判据会在两个靶里各跑一遍
//! （`judge-case` 的头注记过同一条）。故同住一台。
//!
//! **照实记（这一台的文件名）**：靶子的根文件叫 `roster.rs` 而不是 `principal.rs`——因为
//! 它要给 `crate::principal::core` 一个**真实的目录模块**（`tests/principal/mod.rs`），
//! 而 `tests/principal.rs` 与 `tests/principal/` 同名会撞（E0761）。
//!
//! # 这一台钉的是什么
//!
//! **名册**（TID → 此刻代表的号）：一 TID 一格、只有装配者写得动、换绑 = 重定起点。
//! **谱系**（号 → 父）：只增不删、下标即号、零号是根、恰好一个根；`heir` 自反且反对称。
//! **转换**（`adopt` / `waive`）：身份只沿自己那一支往下走，或者回到起点（`origin ≼ current`）。
//! **盟籍**（一张两列表）：反着念是同一个关系的两个方向；空盟合法、号铸过就一直在；
//! `enter` / `leave` 幂等；取窗是"号序 + 阈值游标"。

extern crate alloc;

/// 身份那本册子（就是 `crates/protocol/src/principal/core.rs` 那一份，逐字未改）。
///
/// 包一层内联模块只为让 `crate::principal::core` 这个名字成立——盟籍那一份正是这么写它的
/// `use`（在 `protocol` 里它是 `crate::principal::core`，这里逐字同形）。
mod principal;

/// 盟籍那一份（就是 `crates/protocol/src/coalition/core.rs` 那一份，逐字未改）。
#[path = "../../protocol/src/coalition/core.rs"]
mod coalition;
