//! budget — 诊断预算政策（spare 仓容量单源）：trace 环形常驻 + panic 打印峰值。
//!
//! spare 仓容量 = 常驻 ring（每 hart 窗口表 + 事件槽，随 hart 数线性）+ 崩溃打印
//! 峰值预算（与 hart 数无关：dump 已按 hart 平摊，见 trace::hart_rows）。公式经
//! `spare::region_size`（块头 + 对齐开销）与分配器同源——验收断言与预留恒一致。

use crate::memory::allocator::spare;
use crate::runtime::diagnose::trace;

/// 崩溃打印峰值预算（scene 四表 + 平摊 trace 渲染 + 余量）。health 溢出演练按
/// 此值验证：ring 常驻后余量 ≥ 本预算，且失败路径返回 Err。
pub const DUMP_BUDGET: usize = 32 * 1024;

/// spare 仓容量（页对齐）：ring 常驻 + 打印峰值，含分配器块头/对齐开销。
pub fn spare_budget(h: usize) -> usize {
    spare::region_size(trace::ring_bytes(h), DUMP_BUDGET)
}