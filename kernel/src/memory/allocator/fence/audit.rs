//! 护栏层 · audit — 核查侧：多源交叉核对、关机基线、页清残留检查
//!
//! 与 banker/ledger（簿记：写账/读账）相对，本模块是**核查**：拿账本对账单，
//! boot 收尾 `audit()` 三源交叉核对、关机 `check_baseline` 断言任务帧零泄漏、
//! `page_clear` 验页内无活账。只读账本（banker/ledger），不写。
//! 违例统一经 `report` 处置（见 fence/mod）。

#![cfg(debug_assertions)] // debug 构建生效；release 空体零开销

use core::sync::atomic::{AtomicUsize, Ordering};

use crate::lock::OnceLock;

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

/// 关机块账本基线：boot 时 ledger 活跃块地址全量快照。
///
/// 与帧基线（[`FRAME_BASELINE`]）互补：帧公式只验计数，**池内活块泄漏不扰动
/// 帧计数**（池页同时计入 outstanding 与 held_pages，相消）——必须直接对账本。
/// 快照只存地址（差集键）：泄漏块详情（size/kind/site）以关机现场记录为准；
/// 基线集合 = 持久块（boot 时已活跃、永不释放）——「多出」= 泄漏活块、
/// 「缺少」= 持久块被错误释放。
///
/// 运行期可变（SpinLock）：持久块 realloc 搬家（Vec 扩容 = allocate 新 +
/// deallocate 旧，见 fence::on_free 配对注释）时经 [`rehome_baseline`] 迁址
/// ——搬家不构成差集，差集只报真泄漏/真错释。
static LEDGER_BASELINE: OnceLock<crate::lock::SpinLock<alloc::vec::Vec<usize>>> =
    OnceLock::new();

/// 关机帧账基线：boot 时 banker.held 全集快照。
///
/// held 全集含大量**合法持久持有**（页表、窗口预分配、任务帧等），全量枚举是
/// 噪声——只「关机 held − boot held」差集是信号：净增 = 泄漏帧，净减 = 错还帧。
static FRAME_HELD_BASELINE: OnceLock<alloc::vec::Vec<usize>> = OnceLock::new();

/// 基线块成员查询（on_free 当场检用：基线记录后的释放命中基线地址 = 持久块
/// 被错误释放的第一现场——panic 转储带完整调用栈）。
pub(crate) fn is_baseline_block(addr: usize) -> bool {
    let Some(b) = LEDGER_BASELINE.get() else {
        return false; // 基线未记录（boot 早期）：不检
    };
    b.lock().binary_search(&addr).is_ok()
}

/// 基线块迁址（realloc 搬家）：旧地址从基线移除、新地址入基线（保持有序）。
/// 由 fence::on_free 的搬家配对调用（不持 ledger 锁；本锁 exempt 无层级）。
/// new == 0（窗口内无新块记录——不应发生）时只删不插，防 0 污染基线。
pub(crate) fn rehome_baseline(old: usize, new: usize) {
    let Some(b) = LEDGER_BASELINE.get() else { return };
    let mut g = b.lock();
    if let Ok(i) = g.binary_search(&old) {
        g.remove(i);
    }
    if new != 0 && g.binary_search(&new).is_err() {
        let i = g.binary_search(&new).unwrap_err();
        g.insert(i, new);
    }
}

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
    // 块账本基线：boot 时刻活跃块集合快照。for_each 持 Ledger 锁，快照按
    // len 预分配——回调 push 零分配，不重入块分配器（同锁重入 = depend 误报）。
    //
    // 顺序纪律：先物化**全部**快照存储（两个 Vec 自身的分配也 mark 入账），
    // 再收账——快照 Vec 自身即基线成员。若收账后才分配帧快照 Vec，其块不在
    // 基线，关机差集误报为泄漏块（已实证：1304 假泄漏）。
    let mut snap: alloc::vec::Vec<usize> =
        alloc::vec::Vec::with_capacity(super::ledger::LEDGER.len().max(64));
    let mut held: alloc::vec::Vec<usize> = alloc::vec::Vec::with_capacity(
        super::banker::BANKER.held_count().max(64),
    );
    super::ledger::LEDGER.for_each(|addr, _| snap.push(addr));
    snap.sort_unstable();
    assert!(
        LEDGER_BASELINE
            .set(crate::lock::SpinLock::new(snap))
            .is_ok(),
        "ledger baseline double-recorded"
    );
    // 帧账基线：banker.held 全集快照（collect_held 无锁位图逐字读，零分配——
    // 由调用方传预分配 Vec，已在上方随块快照一并物化）。
    super::banker::BANKER.collect_held(&mut held);
    held.sort_unstable();
    assert!(
        FRAME_HELD_BASELINE.set(held).is_ok(),
        "frame-held baseline double-recorded"
    );
}

/// 断言关机时任务帧已全部归还（非块池在途帧 == 基线）。
///
/// 公式取「帧 − 块池页」差，剔除块池波动，专验任务地址空间 Drop 的正确性。
#[track_caller]
pub fn check_baseline() {
    // ── 块账本差集（先于帧公式）──────────────────────────────
    // 帧公式对池内活块泄漏是盲的（池页 outstanding/held_pages 相消），此处直接
    // 对账本：关机时 ledger 应回到 boot 基线集合（realloc 搬家已在 fence 侧
    // rehome，不构成差集）。只打印不 report——随后帧公式检查照跑，两路信息都
    // 落屏后统一违例（见函数尾）。
    //
    // 锁序：for_each 持 Ledger 锁，收集 Vec 按 len 预分配——回调 push 零分配，
    // 不重入块分配器（同锁重入 = depend 误报 panic）。
    let mut now_recs: alloc::vec::Vec<(usize, usize, OwnerKind, usize, usize)> =
        alloc::vec::Vec::with_capacity(super::ledger::LEDGER.len().max(64));
    super::ledger::LEDGER.for_each(|addr, rec| {
        now_recs.push((addr, rec.size, rec.kind, rec.site, rec.site2));
    });
    now_recs.sort_unstable_by(|a, b| a.0.cmp(&b.0));
    let mut leaked = 0usize;
    let mut missing = 0usize;
    {
        // 基线锁只包差集计算（putln/report 在锁外）。
        let base_recs = LEDGER_BASELINE
            .get()
            .expect("ledger baseline not recorded")
            .lock();
        for &(addr, size, kind, site, site2) in &now_recs {
            if base_recs.binary_search(&addr).is_err() {
                if leaked == 0 {
                    crate::putln!("[audit] leaked blocks at shutdown:");
                }
                crate::putln!(
                    "  leak[{leaked}] = {addr:#x} size {size} kind {kind:?} site {site:#x}/{site2:#x}"
                );
                leaked += 1;
            }
        }
        for &addr in base_recs.iter() {
            if now_recs.binary_search_by(|r| r.0.cmp(&addr)).is_err() {
                if missing == 0 {
                    crate::putln!("[audit] missing baseline blocks at shutdown:");
                }
                crate::putln!("  missing[{missing}] = {addr:#x}");
                missing += 1;
            }
        }
    }

    // ── 帧账差集（纯诊断，不判违规）────────────────────────────
    // held 全集含大量合法持久持有（页表、窗口预分配、任务帧等），全量枚举是
    // 噪声（基线即有 ~150 页）；「关机 − boot」差集有信号也有合法周转伪影：
    // 净增 = 泄漏帧**或**运行期新建持久页（页表中间节点）；净减 = 错还帧**或**
    // 窗口 slot 页正常周转。判定仍由计数公式（下）——差集只提供定位线索。
    let mut now_held: alloc::vec::Vec<usize> = alloc::vec::Vec::with_capacity(
        super::banker::BANKER.held_count().max(64),
    );
    super::banker::BANKER.collect_held(&mut now_held);
    now_held.sort_unstable();
    let base_held = FRAME_HELD_BASELINE.get().expect("frame-held baseline not recorded");
    let mut f_leaked = 0usize;
    for &pa in &now_held {
        if base_held.binary_search(&pa).is_err() {
            if f_leaked == 0 {
                crate::putln!("[audit] leaked frames at shutdown:");
            }
            crate::putln!("  frame_leak[{f_leaked}] = {pa:#x}");
            f_leaked += 1;
        }
    }
    let mut f_missing = 0usize;
    for &pa in base_held {
        if now_held.binary_search(&pa).is_err() {
            if f_missing == 0 {
                crate::putln!("[audit] missing baseline frames at shutdown:");
            }
            crate::putln!("  frame_missing[{f_missing}] = {pa:#x}");
            f_missing += 1;
        }
    }

    let now = crate::memory::allocator::frame::outstanding();
    let blocks = crate::memory::allocator::block::held_pages();
    let base = FRAME_BASELINE.load(Ordering::Relaxed);
    let bbase = BLOCK_BASELINE.load(Ordering::Relaxed);
    // wrapping_sub：防回归早期 `now < blocks` 下溢（现全部实现已保证不越，防御性）。
    let lhs = now.wrapping_sub(blocks);
    let rhs = base.wrapping_sub(bbase);
    if lhs != rhs {
        // 帧差集已逐条落屏（见上）；此处计数公式兜底（差集在两者相等时也触发
        // 不了——如「借一还一」的同数置换，但 banker 位图差集已覆盖该形态）。
        report(
            IntegrityViolation::AuditDivergence,
            0,
            format_args!(
                "task frames leaked at shutdown: outstanding {now} (blocks {blocks}) != baseline {base} (blocks {bbase})"
            ),
        );
    }
    // 块差集违例（详情已逐条落屏）：池内泄漏是帧公式的盲区，此处兜底。
    // 帧差集不判违规（周转伪影，见上注释）——帧违例由计数公式（上）判定。
    if leaked > 0 || missing > 0 {
        report(
            IntegrityViolation::AuditDivergence,
            0,
            format_args!("ledger diverged at shutdown: {leaked} leaked, {missing} missing"),
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
