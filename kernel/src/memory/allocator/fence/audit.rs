//! 护栏层 · audit — 核查侧：多源交叉核对、关机基线、页清残留检查
//!
//! 与 banker/ledger（簿记：写账/读账）相对，本模块是**核查**：拿账本对账单，
//! boot 收尾 `audit()` 三源交叉核对、关机 `check_baseline` 断言任务帧零泄漏、
//! `page_clear` 验页内无活账。只读账本（banker/ledger），不写。
//! 违例统一经 `report` 处置（见 fence/mod）。

#![cfg(debug_assertions)] // debug 构建生效；release 空体零开销

use crate::lock::OnceLock;
use crate::memory::manager::addr::PhysAddr;

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

/// 关机块账本基线：boot 时 ledger 活跃块地址全量快照。
///
/// 帧计数对**池内活块泄漏是盲的**（池页同时计入 outstanding 与 held_pages，
/// 相消）——必须直接对账本。快照只存地址（差集键）：泄漏块详情（size/kind/site）
/// 以关机现场记录为准；基线集合 = 持久块（boot 时已活跃、永不释放）——
/// 「多出」= 泄漏活块、「缺少」= 持久块被错误释放。
///
/// 运行期可变（SpinLock）：持久块 realloc 搬家（Vec 扩容 = allocate 新 +
/// deallocate 旧，见 fence::on_free 配对注释）时经 [`rehome_baseline`] 迁址
/// ——搬家不构成差集，差集只报真泄漏/真错释。
static LEDGER_BASELINE: OnceLock<crate::lock::SpinLock<alloc::vec::Vec<usize>>> =
    OnceLock::new();

/// 关机帧身份基线（三集合，boot 快照；均排序，差集用）：
///   POOL        — boot 块池持页全集（`block::collect_owned_pa`）。
///   TABLES      — boot 内核页表树表页全集（[`collect_kernel_tables`]）。
///   PERSISTENT  — boot held − POOL − TABLES：内核持久帧（trap 帧/栈、trampoline、
///                 boot 窗口预分配、health 遗留等——永不释放类）。
///
/// 计数公式（outstanding − blocks == 基线同式）已废弃：块池持页是自由变量
/// （pool drain/top-up 正常周转——health 遗留 8 页运行期归还即假违规），且同数
/// 置换盲。改按**帧身份**分区核对（见 [`check_baseline`]）。
static POOL_BASELINE: OnceLock<alloc::vec::Vec<usize>> = OnceLock::new();
static TABLES_BASELINE: OnceLock<alloc::vec::Vec<usize>> = OnceLock::new();
static PERSISTENT_BASELINE: OnceLock<alloc::vec::Vec<usize>> = OnceLock::new();

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

/// 收集内核根页表树可达的全部页表页 PA（只下钻非叶 PTE；恒等映射叶不收）。
///
/// 内核恒等映射整个 DRAM（boot 装配）——「内核引用集」因此包含全部 DRAM 页，
/// 不能用于区分任务帧；但**表页**仍可区分：任务空间表挂在各自 satp 根下
/// （不经内核根可达），关机时全部任务已退、任务表应已随 Drop 归还——本函数
/// 只收内核树（根 + 中间表）。逐级裸读 PTE（walk_raw 同源守卫）：表 PA 均过
/// in_dram 校验后才读；下钻深度以 mode::levels() 封顶，环状坏表有限步终止。
fn collect_kernel_tables(out: &mut alloc::vec::Vec<usize>) {
    fn descend(
        tbl: usize,
        level: usize,
        ok: &dyn Fn(PhysAddr) -> bool,
        out: &mut alloc::vec::Vec<usize>,
    ) {
        out.push(tbl);
        if level == 0 {
            return; // 叶层无子表（防御；正常路径 level > 0 才下钻）
        }
        for idx in 0..512usize {
            // SAFETY: tbl 已过 in_dram 校验（入口根也校验）；S 态直读恒等映射。
            let pte = unsafe {
                *((tbl + idx * 8) as *const crate::memory::manager::entry::PageTableEntry)
            };
            if !pte.is_valid() || pte.is_leaf() {
                continue;
            }
            let child = pte.paddr() as usize;
            if ok(PhysAddr::from_raw(child)) {
                descend(child, level - 1, ok, out);
            }
        }
    }
    let satp_val = riscv::register::satp::read().bits();
    let root = (satp_val & ((1usize << 44) - 1)) << 12;
    let ok = |pa: PhysAddr| {
        (0x8000_0000..crate::machine::dram_edge().unwrap_or(0x9000_0000))
            .contains(&pa.as_usize())
    };
    if ok(PhysAddr::from_raw(root)) {
        descend(root, crate::memory::manager::mode::levels(), &ok, out);
    }
}

/// 记录基线：此后在途帧应只增任务所有，关机时全部归还。
pub fn record_baseline() {
    // 块账本基线：boot 时刻活跃块集合快照。for_each 持 Ledger 锁，快照按
    // len 预分配——回调 push 零分配，不重入块分配器（同锁重入 = depend 误报）。
    //
    // 顺序纪律：先物化**全部**快照存储（各 Vec 自身的分配也 mark 入账），
    // 再收账——快照 Vec 自身即基线成员。若收账后才分配帧快照 Vec，其块不在
    // 基线，关机差集误报为泄漏块（已实证：1304 假泄漏）。
    let mut snap: alloc::vec::Vec<usize> =
        alloc::vec::Vec::with_capacity(super::ledger::LEDGER.len().max(64));
    let mut held: alloc::vec::Vec<usize> = alloc::vec::Vec::with_capacity(
        super::banker::BANKER.held_count().max(64),
    );
    let mut pool: alloc::vec::Vec<usize> = alloc::vec::Vec::with_capacity(
        super::banker::BANKER.held_count().max(64),
    );
    let mut tables: alloc::vec::Vec<usize> = alloc::vec::Vec::with_capacity(
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
    // 帧身份基线：held 全集（collect_held 无锁位图逐字读，零分配——由调用方
    // 传预分配 Vec，已在上方随块快照一并物化）→ 按身份三分。
    super::banker::BANKER.collect_held(&mut held);
    crate::memory::allocator::block::collect_owned_pa(&mut pool);
    collect_kernel_tables(&mut tables);
    pool.sort_unstable();
    tables.sort_unstable();
    held.sort_unstable();
    assert!(POOL_BASELINE.set(pool).is_ok(), "pool baseline double-recorded");
    assert!(
        TABLES_BASELINE.set(tables).is_ok(),
        "tables baseline double-recorded"
    );
    // 持久帧 = held − pool − tables（boot 在途的全部非池非表帧：trap 帧/栈、
    // trampoline、窗口预分配等永不释放类）。retain 就地收缩，零分配。
    let pool_b = POOL_BASELINE.get().expect("pool baseline set above");
    let tables_b = TABLES_BASELINE.get().expect("tables baseline set above");
    held.retain(|p| {
        pool_b.binary_search(p).is_err() && tables_b.binary_search(p).is_err()
    });
    assert!(
        PERSISTENT_BASELINE.set(held).is_ok(),
        "persistent baseline double-recorded"
    );
}

/// 断言关机时任务帧已全部归还（帧身份分区核对）。
///
/// 计数公式（outstanding − blocks == 基线同式）已废弃：块池持页自由周转
/// （pool drain/top-up 合法——health 遗留 8 页运行期归还即假违规），且同数
/// 置换盲。关机 held 全集按**帧身份**分区与 boot 基线核对：
///   ① 孤儿     — held − pool − tables − persistent：任务生命周期帧（栈槽/
///                窗口/trap 帧/用户空间表）全部任务已退仍未归还 = 真泄漏 → 违规。
///   ② 持久缺失 — persistent − held：boot 持久帧被错误归还 → 违规。
///   ③ 表页     — 净增打印（任务遗留子树，① 已报违规，此处定位）；净减违规
///               （boot 表页被摘——恒等映射区表永不空，不应发生）。
///   ④ 块池页   — 计数诊断（正常周转，非违规）；池内活块泄漏由块账本差集
///               （上）判定。
///   ⑤ 一致性   — banker held 与 frame outstanding 必须相符（簿记不变量）。
#[track_caller]
pub fn check_baseline() {
    // ── 块账本差集（先于帧类检查）──────────────────────────────
    // 帧类检查对池内活块泄漏是盲的（池页 outstanding/held_pages 相消），此处直接
    // 对账本：关机时 ledger 应回到 boot 基线集合（realloc 搬家已在 fence 侧
    // rehome，不构成差集）。只打印不 report——随后帧类检查照跑，两路信息都
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

    // ── 帧身份分区核对 ─────────────────────────────────────
    // 收集存储预物化（容量 ≥ 全集，push 零分配）；banker 位图 / 块池 / 页表
    // 三源收集互不持锁重叠（collect_owned_pa 的池锁在 ledger 锁外）。
    let mut now_held: alloc::vec::Vec<usize> = alloc::vec::Vec::with_capacity(
        super::banker::BANKER.held_count().max(64),
    );
    let mut now_pool: alloc::vec::Vec<usize> = alloc::vec::Vec::with_capacity(
        super::banker::BANKER.held_count().max(64),
    );
    let mut now_tables: alloc::vec::Vec<usize> = alloc::vec::Vec::with_capacity(
        super::banker::BANKER.held_count().max(64),
    );
    super::banker::BANKER.collect_held(&mut now_held);
    crate::memory::allocator::block::collect_owned_pa(&mut now_pool);
    collect_kernel_tables(&mut now_tables);
    now_held.sort_unstable();
    now_pool.sort_unstable();
    now_tables.sort_unstable();
    let pool_base = POOL_BASELINE.get().expect("pool baseline not recorded");
    let tables_base = TABLES_BASELINE.get().expect("tables baseline not recorded");
    let persistent = PERSISTENT_BASELINE
        .get()
        .expect("persistent baseline not recorded");

    // ① 孤儿：held 中不属于任一身份类的帧——任务生命周期帧未归还（真泄漏）。
    let mut orphans = 0usize;
    for &pa in &now_held {
        if now_pool.binary_search(&pa).is_err()
            && now_tables.binary_search(&pa).is_err()
            && persistent.binary_search(&pa).is_err()
        {
            if orphans == 0 {
                crate::putln!("[audit] orphan task frames at shutdown:");
            }
            crate::putln!("  orphan[{orphans}] = {pa:#x}");
            orphans += 1;
        }
    }
    // ② 持久缺失：boot 持久帧被错误归还。
    let mut freed_persistent = 0usize;
    for &pa in persistent {
        if now_held.binary_search(&pa).is_err() {
            if freed_persistent == 0 {
                crate::putln!("[audit] freed persistent frames at shutdown:");
            }
            crate::putln!("  freed[{freed_persistent}] = {pa:#x}");
            freed_persistent += 1;
        }
    }
    // ③ 表页差集：净增 = 任务遗留子树（① 已报违规，此处定位线索）；净减 =
    // boot 表页被摘（恒等映射区表永不空——真错还）。
    let mut tbl_grown = 0usize;
    for &pa in &now_tables {
        if tables_base.binary_search(&pa).is_err() {
            if tbl_grown == 0 {
                crate::putln!("[audit] grown page-table pages at shutdown:");
            }
            crate::putln!("  table_grow[{tbl_grown}] = {pa:#x}");
            tbl_grown += 1;
        }
    }
    let mut tbl_shrunk = 0usize;
    for &pa in tables_base {
        if now_tables.binary_search(&pa).is_err() {
            if tbl_shrunk == 0 {
                crate::putln!("[audit] missing baseline table pages at shutdown:");
            }
            crate::putln!("  table_missing[{tbl_shrunk}] = {pa:#x}");
            tbl_shrunk += 1;
        }
    }
    // ④ 块池页：正常周转，计数诊断。
    crate::putln!(
        "[audit] block-pool pages: {} -> {} (turnover, not a violation)",
        pool_base.len(),
        now_pool.len()
    );

    // ⑤ 一致性：banker 位图与 frame 计数器必须相符（簿记不变量）。
    let held = now_held.len();
    let outstanding = crate::memory::allocator::frame::outstanding();
    if held != outstanding {
        report(
            IntegrityViolation::AuditDivergence,
            0,
            format_args!("banker held {held} != frame outstanding {outstanding} at shutdown"),
        );
    }

    // 违例汇总：块账本差集（池内泄漏/错释）+ 帧身份（孤儿/持久错还/表页被摘）。
    if leaked > 0 || missing > 0 {
        report(
            IntegrityViolation::AuditDivergence,
            0,
            format_args!("ledger diverged at shutdown: {leaked} leaked, {missing} missing"),
        );
    }
    if orphans > 0 || freed_persistent > 0 || tbl_shrunk > 0 {
        report(
            IntegrityViolation::AuditDivergence,
            0,
            format_args!(
                "frame identity diverged at shutdown: {orphans} orphan, {freed_persistent} freed-persistent, {tbl_shrunk} table-missing"
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
