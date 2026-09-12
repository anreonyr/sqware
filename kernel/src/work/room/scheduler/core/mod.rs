// 调度核心（scheduler::core）— per-hart 调度：纯功能，无适配代码。
//
// 四文件按接缝分（自包含、可独立推理，不依赖任何具体调用方）：
//   hart.rs   本机调度器：容器（running + 就绪队列）、装槽 / 让位 / 轮转 / 续跑
//   ident.rs  本核身份槽 `Badge` + 身份读法 `ident()`
//   table.rs  全局表（per-hart 调度器数组）+ 名册 + 全机扫描 + 关机终末释放
//   fetch.rs  取活：跨核偷取 + WFI 休眠
//
// 适配面（`scheduler/{boot,trap}.rs`）只经本文件触及核心：本模块内跨到 `scheduler`
// 一级的条目取 `pub(in super::super)`——刚好到 `scheduler`，不放宽到 `pub(crate)`
// （同 `messenger/wait/{site,holder}.rs` 的纪律）；只在本核心内部用的条目取
// `pub(super)`（= 到 `core`）。
//
// 外部路径不变：`scheduler::core::X` 照旧（重导出于下）。

pub(super) mod fetch;
pub(super) mod hart;
pub(super) mod ident;
pub(super) mod table;

// 入口面（boot / trap）借用的表面。
pub(super) use fetch::fetch;
pub(super) use hart::Scheduler;
pub(super) use table::SCHEDULERS;

// scheduler 之外消费的表面（messenger / envcall / unit / diagnose）。
pub use ident::{Identity, ident};
#[cfg(feature = "audit")]
pub(crate) use table::roster_live;
pub(crate) use table::{
    current, enlist, launch, muster, remove_from_starved, rip, roster, running_hart,
    try_reserve_roster, try_reserve_starved,
};
