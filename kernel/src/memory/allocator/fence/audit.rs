//! 护栏层 · audit — 核查侧：多源交叉核对、关机基线、页清残留检查
//!
//! 与 banker/ledger（簿记：写账/读账）相对，本模块是**核查**：拿账本对账单，
//! boot 收尾 `audit()` 三源交叉核对、关机 `check_baseline` 断言任务帧零泄漏、
//! `page_clear` 验页内无活账。只读账本（banker/ledger），不写。
//! 违例统一经 `report` 处置（见 fence/mod）。

#![cfg(all(debug_assertions, feature = "audit"))] // 与 fence 根同 gate（debug + audit）

use core::sync::atomic::{AtomicUsize, Ordering};

use super::{IntegrityViolation, ledger::OwnerKind, report};

// ── 基线核算 ──────────────────────────────────────────

/// 基线核算快照（check_baseline / 启动横幅用）。
pub struct IntegrityStats {
    pub held_frames: usize,
    pub kernel_blocks: usize,
    pub user_blocks: usize,
}

pub fn stats() -> IntegrityStats {
    let mut kernel_blocks = 0usize;
    let mut user_blocks = 0usize;
    super::ledger::LEDGER.for_each(|_, rec| match rec.kind {
        OwnerKind::KernelHeap => kernel_blocks += 1,
        OwnerKind::UserHeap => user_blocks += 1,
    });
    IntegrityStats {
        held_frames: super::banker::BANKER.held_count(),
        kernel_blocks,
        user_blocks,
    }
}

// ── 基线 ──────────────────────────────────────────────

/// 内核持久帧基线（boot 记录；关机断言回落）。全动态下块池持页已计入
/// frame outstanding，比较时剔除——只验「非块池在途帧」回到基线。
static FRAME_BASELINE: AtomicUsize = AtomicUsize::new(0);
static BLOCK_BASELINE: AtomicUsize = AtomicUsize::new(0);

/// 记录基线（在 spawn 用户任务**之前**调用）——此后在途帧应只增用户任务所有，
/// 关机时全部归还；断言触发 = 任务地址空间/栈所有权 Drop 有泄漏。
pub fn record_baseline() {
    FRAME_BASELINE.store(
        crate::memory::allocator::frame::outstanding(),
        Ordering::Relaxed,
    );
    BLOCK_BASELINE.store(
        crate::memory::allocator::block::held_pages(),
        Ordering::Relaxed,
    );
}

/// 断言关机时任务帧已全部归还（非块池在途帧 == 基线）。
///
/// 全动态下块池持页数随任务分配波动：关机 flush_all 已把空闲页全还、忙页
/// 保留（内核常驻 + 任务残留）。公式取「帧 − 块池页」差，剔除块池波动，
/// 专验任务地址空间 Drop 的正确性（泄漏的块页使块池持页数高于基线，差变小）。
#[track_caller]
pub fn check_baseline() {
    let now = crate::memory::allocator::frame::outstanding();
    let blocks = crate::memory::allocator::block::held_pages();
    let base = FRAME_BASELINE.load(Ordering::Relaxed);
    let bbase = BLOCK_BASELINE.load(Ordering::Relaxed);
    if now - blocks != base - bbase {
        report(
            IntegrityViolation::AuditDivergence,
            0,
            format_args!(
                "task frames leaked at shutdown: outstanding {now} (blocks {blocks}) != baseline {base} (blocks {bbase})"
            ),
        );
    }
}

// ── 审计 ──────────────────────────────────────────────

/// 整页清链后的记账完整性检查：页内须无活账目方可返回（有 → report）。
pub fn page_clear(pa: usize) {
    let mut any = false;
    super::ledger::LEDGER.for_each(|addr, _| {
        if addr >= pa && addr < pa + crate::memory::PAGE_SIZE {
            any = true;
        }
    });
    if any {
        report(
            IntegrityViolation::AuditDivergence,
            pa,
            format_args!("page returned with live ledger entries"),
        );
    }
}

/// 全量审计（boot 收尾调用一次；三源交叉核对，违例即 report）：
///   Banker.held_count == frame.outstanding（块池页已计入 frame outstanding——
///   prime 借页时 frame 已 debit、drain 还页时 frame 已 credit，block 不重复记账）；
///   每条 KernelHeap 记录地址须落在某池持有页（页头 MAGIC 判定），且所在页须 held。
pub fn audit() {
    let held = super::banker::BANKER.held_count();
    let frames = crate::memory::allocator::frame::outstanding();
    if held != frames {
        report(
            IntegrityViolation::AuditDivergence,
            0,
            format_args!("banker {held} != frames {frames}"),
        );
    }
    super::ledger::LEDGER.for_each(|addr, rec| {
        let page = addr & !(crate::memory::PAGE_SIZE - 1);
        if rec.kind == OwnerKind::KernelHeap {
            if !crate::memory::allocator::block::pool_includes(addr) {
                report(
                    IntegrityViolation::WildAddress,
                    addr,
                    format_args!("kernel-heap record outside block-owned pages"),
                );
            }
            if !super::banker::BANKER.is_held(page) {
                report(
                    IntegrityViolation::AuditDivergence,
                    addr,
                    format_args!("kernel-heap record on non-held page {page:#x}"),
                );
            }
        } else if !super::banker::BANKER.is_held(page) {
            report(
                IntegrityViolation::AuditDivergence,
                addr,
                format_args!("user-heap record VA on non-held page {page:#x}"),
            );
        }
    });
}