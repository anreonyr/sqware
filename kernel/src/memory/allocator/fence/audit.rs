//! 护栏层 · audit — 核查侧：多源交叉核对、关机基线、页清残留检查
//!
//! 与 banker/ledger（簿记：写账/读账）相对，本模块是**核查**：拿账本对账单，
//! boot 收尾 `audit()` 三源交叉核对、关机 `check_baseline` 断言任务帧零泄漏、
//! `page_clear` 验页内无活账。只读账本（banker/ledger），不写。
//! 违例统一经 `report` 处置（见 fence/mod）。

#![cfg(debug_assertions)] // debug 构建生效；release 空体零开销

use core::sync::atomic::{AtomicUsize, Ordering};

use super::{IntegrityViolation, OwnerKind, report};

// ── 基线核算 ──────────────────────────────────────────

/// 基线核算快照。
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

/// 记录基线：此后在途帧应只增任务所有，关机时全部归还。
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
/// 公式取「帧 − 块池页」差，剔除块池波动，专验任务地址空间 Drop 的正确性。
#[track_caller]
pub fn check_baseline() {
    let now = crate::memory::allocator::frame::outstanding();
    let blocks = crate::memory::allocator::block::held_pages();
    let base = FRAME_BASELINE.load(Ordering::Relaxed);
    let bbase = BLOCK_BASELINE.load(Ordering::Relaxed);
    // wrapping_sub：防回归早期 `now < blocks` 下溢（现全部实现已保证不越，防御性）。
    let lhs = now.wrapping_sub(blocks);
    let rhs = base.wrapping_sub(bbase);
    if lhs != rhs {
        // 诊断：枚举游离帧（`banker.held - block.owned`）— 逐条 PA 打印，
        // 把"差几帧"具体到"哪几页没归还"，便于按 VA/类别定位泄漏源。
        // 锁序：先收 banker.held（无锁位图，逐字 Relaxed 读），再收 block.owned
        // （tally 锁内）。owned/strays 必须**预分配**——collect_owned_pa 持
        // tally 锁时 Vec::push 触发的 alloc 会重入 tally 锁（同锁重入 = depend
        // 误报 panic）；预分配到 held_count 上界可让 push 不再调 alloc。
        let mut strays: alloc::vec::Vec<usize> = alloc::vec::Vec::new();
        {
            // 预分配 held 容量 = banker 全集大小（held_count() 上界），避免
            // collect_held 内 push 多次 realloc。release 下 held_count() = 0，
            // 故用 `with_capacity` 给一个宽松上界即可。
            let cap = super::banker::BANKER.held_count().max(64);
            let mut held: alloc::vec::Vec<usize> = alloc::vec::Vec::with_capacity(cap);
            super::banker::BANKER.collect_held(&mut held);
            let mut owned: alloc::vec::Vec<usize> =
                alloc::vec::Vec::with_capacity(held.len());
            crate::memory::allocator::block::collect_owned_pa(&mut owned);
            owned.sort_unstable();
            strays.reserve(held.len());
            for &pa in &held {
                if owned.binary_search(&pa).is_err() {
                    strays.push(pa);
                }
            }
        }
        crate::putln!(
            "[audit] {} stray frame(s) at shutdown:",
            strays.len()
        );
        for (i, pa) in strays.iter().enumerate() {
            crate::putln!("  stray[{i}] = {pa:#x}");
        }
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
///   Banker.held_count == frame.outstanding；
///   每条 KernelHeap 记录地址须落在某池持有页，且所在页须 held。
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
