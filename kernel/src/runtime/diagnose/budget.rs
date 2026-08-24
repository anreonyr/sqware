//! budget — 诊断预算政策（spare 仓容量单源）：trace 环形常驻 + panic 打印峰值。
//!
//! spare 仓容量 = 常驻 ring（每 hart 窗口表 + 事件槽，随 hart 数线性）+ 崩溃打印
//! 峰值预算（与 hart 数无关：dump 已按 hart 平摊，见 trace::hart_rows）。公式经
//! `spare::region_size`（块头 + 对齐开销）与分配器同源——验收断言与预留恒一致。

use crate::memory::allocator::spare;
use crate::runtime::diagnose::trace;

/// 崩溃打印峰值预算。**实测校准（panic E2E）**：stanza 渲染瞬态风暴规模随预算
/// 同向浮动（多轮 96→512KiB 校准中峰值始终贴当时 cap、余量 <1KB，疑似渲染器
/// 按可用内存上限分配，未及深挖）——故定 **1MiB**：dump 内容天然有界（表行数
/// 编译期封顶），海量余量下任何栈形/seed 都安全；128MB DRAM 中占比 <1%。
/// `[spare]` 行自报 used/peak 供审计（预算即契约的证据面）。health 溢出演练按
/// 此值验证：ring 常驻后余量 ≥ 本预算，且失败路径返回 Err。
pub const DUMP_BUDGET: usize = 1024 * 1024;

/// spare 仓容量（页对齐）：ring 常驻 + 打印峰值，含分配器块头/对齐开销。
pub fn spare_budget(h: usize) -> usize {
    spare::region_size(trace::ring_bytes(h), DUMP_BUDGET)
}