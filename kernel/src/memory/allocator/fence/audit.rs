//! 护栏层 · audit — 核查侧：所有权类别记账、关机不变量、页清残留检查
//!
//! 与 banker/ledger（簿记：写账/读账）相对，本模块是**核查**：拿类别计数对
//! 不变量。只读账本（banker/ledger/类别计数），不写。违例统一经 `report`
//! 处置（见 fence/mod）。
//!
//! # 设计：所有权类别记账（替代旧「boot 身份快照 vs 关机差集」）
//!
//! 每帧/每块按生命周期归属一个类别（[`FrameClass`] / [`BlockClass`]），计数由
//! fence 事件入口维护（帧类别存 pagemeta、块类别存 ledger 记录）。合法形态
//! 演化——容器扩容、realloc 搬家（类别继承）、池页周转、审计工具自身分配
//! （Audit 类豁免）——只是类别内部的变化，**不需要任何赦免机制**。旧框架的
//! 基线快照（LEDGER/POOL/TABLES/PERSISTENT）、rehome_baseline / adopt_baseline、
//! 快照余量、AUDITING 豁免标志全部删除（1634c36 教训：快照物化 prime 自扰 →
//! mid-collection realloc → 孤儿帧——类别记账从根上免疫该类问题：审计期分配
//! 是 Audit 类，不参与任何归零检查）。
//!
//! 关机检查 [`check_baseline`] 五步：
//!   ① TASK_FRAMES == 0 && TASK_BLOCKS == 0   真泄漏（替代旧孤儿 + 块差集）。
//!   ② 持久注册表逐项仍 held                   持久错还（替代旧持久缺失差集）。
//!   ③ TABLE_FRAMES == 内核根表 walk 数         任务表遗留 / 内核表被摘。
//!   ④ 池页计数诊断（周转，非违规）。
//!   ⑤ banker held == frame outstanding         簿记不变量。
//!
//! boot 收尾 [`audit()`] 三源交叉核对 + 类别计数 sanity；[`page_clear`] 验页内
//! 无活账。

#![cfg(feature = "audit")] // audit feature（debug 默认开；release 可显式 --features audit）

use core::alloc::Allocator;
use core::sync::atomic::Ordering;

use crate::lock::OnceLock;
use crate::memory::manager::addr::PhysAddr;

use super::{BlockClass, FrameClass, IntegrityViolation, OwnerKind, report};

// ── 统计 ──────────────────────────────────────────────

/// 核算快照（类别计数 + ledger 分布；诊断用）。
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

// ── 持久注册表 ────────────────────────────────────────

/// boot 持久帧登记：显式 **add-only** 注册 `(pa, name)`——trap 栈块、spare 仓块、
/// 内核窗口帧（各自 boot 初始化点注册）。硬规则：持久帧永不移动（移动的是缓冲，
/// 缓冲是块/池页）——注册表永远有效，无 rehome/adopt。关机逐项校验仍 held
/// （[`banker::is_held`]），错还即违例（替代旧「持久缺失差集」——旧差集把
/// boot held 全集当持久集，任何合法归还都需赦免；注册表只覆盖**声明持久**的
/// 少量结构，其余 boot 期分配属默认类 Persistent，不参与检查）。
struct PersistEntry {
    pa: usize,
    name: &'static str,
}

static PERSISTENT: OnceLock<crate::lock::SpinLock<alloc::vec::Vec<PersistEntry>>> =
    OnceLock::new();

/// 登记持久帧（boot 调用；add-only）。`pa` = 帧块基址（分配事件首地址——banker
/// held 位与类别记账均按分配事件首页）。
#[cfg(feature = "audit")]
pub(crate) fn register_persistent(pa: usize, name: &'static str) {
    let list = PERSISTENT.get_or_init(|| crate::lock::SpinLock::new(alloc::vec::Vec::new()));
    list.lock().push(PersistEntry { pa, name });
}

// ── 表页 walk ─────────────────────────────────────────

/// 收集内核根页表树可达的全部页表页 PA（只下钻非叶 PTE；恒等映射叶不收）。
///
/// 内核恒等映射整个 DRAM（boot 装配）——「内核引用集」因此包含全部 DRAM 页，
/// 不能用于区分任务帧；但**表页**仍可区分：任务空间表挂在各自 satp 根下
/// （不经内核根可达），关机时全部任务已退、任务表应已随 Drop 归还——本函数
/// 只收内核树（根 + 中间表）。逐级裸读 PTE（walk_raw 同源守卫）：表 PA 均过
/// in_dram 校验后才读；下钻深度以 mode::levels() 封顶，环状坏表有限步终止。
fn collect_kernel_tables(out: &mut alloc::vec::Vec<usize, &'static dyn Allocator>) {
    fn descend(
        tbl: usize,
        level: usize,
        ok: &dyn Fn(PhysAddr) -> bool,
        out: &mut alloc::vec::Vec<usize, &'static dyn Allocator>,
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

// ── 关机检查 ──────────────────────────────────────────

/// 断言关机时任务生命周期帧/块已全部归还（类别记账五步，见模块头）。
///
/// 与旧框架（boot 快照差集）不同：本检查对「合法形态演化」天然免疫——容器
/// 扩容、realloc 搬家（新块继承类别）、池页周转、审计工具自身分配（Audit 类
/// 豁免）都只是类别内部的变化，不构成违规、不需要赦免。
#[track_caller]
pub fn check_baseline() {
    // 审计期标记：仅块池 drain 守卫消费（见 block::drain 注释）。
    super::AUDITING.store(true, Ordering::Relaxed);

    // 收集存储物化（Audit 类：记账豁免——审计工具不自扰归零检查/簿记核对）。
    // 容量 ≥ 全集（held_count 上界），push 零分配、无 realloc（关机单核无并发
    // 分配）；即使 realloc，新缓冲经打标分配器同样 Audit 类。
    let mut now_tables: alloc::vec::Vec<usize, &'static dyn Allocator> =
        alloc::vec::Vec::with_capacity_in(
            super::banker::BANKER.held_count().max(64),
            crate::memory::allocator::block::tag_audit(),
        );
    let mut now_pool: alloc::vec::Vec<usize, &'static dyn Allocator> =
        alloc::vec::Vec::with_capacity_in(
            super::banker::BANKER.held_count().max(64),
            crate::memory::allocator::block::tag_audit(),
        );

    // ① 任务类泄漏：帧/块类别计数归零（真泄漏判据——替代旧孤儿 + 块差集）。
    let task_frames = super::frame_count(FrameClass::Task);
    let task_blocks = super::block_count(BlockClass::Task);
    if task_frames > 0 || task_blocks > 0 {
        crate::putln!(
            "[audit] task lifecycle leak at shutdown: {task_frames} frames, {task_blocks} blocks"
        );
    }

    // ② 持久注册表：逐项仍 held（持久帧错还 = 违例）。
    let mut freed_persistent = 0usize;
    if let Some(reg) = PERSISTENT.get() {
        let g = reg.lock();
        for e in g.iter() {
            if !super::banker::BANKER.is_held(e.pa) {
                if freed_persistent == 0 {
                    crate::putln!("[audit] freed persistent frames at shutdown:");
                }
                crate::putln!("  freed[{freed_persistent}] = {} @ {:#x}", e.name, e.pa);
                freed_persistent += 1;
            }
        }
    }

    // ③ 表页计数 vs 内核根表 walk：TABLE_FRAMES（类别计数）与当前内核树表页
    // 数必须相符——任务表遗留（计数 > walk）或内核表被摘（计数 < walk）皆违例。
    collect_kernel_tables(&mut now_tables);
    let walk_tables = now_tables.len();
    let table_frames = super::frame_count(FrameClass::Table);
    if table_frames != walk_tables {
        crate::putln!(
            "[audit] table frames {table_frames} != kernel-walk count {walk_tables}:"
        );
        for (i, &pa) in now_tables.iter().take(16).enumerate() {
            crate::putln!("  walk[{i}] = {pa:#x}");
        }
    }

    // ④ 块池页：计数诊断（正常周转，非违规——池页属 Pool 类自由周转）。
    crate::memory::allocator::block::collect_owned_pa(&mut now_pool);
    let pool_pages = now_pool.len();
    crate::putln!(
        "[audit] block-pool pages: {pool_pages} (turnover, not a violation)"
    );

    // ⑤ 一致性：banker held 与 frame outstanding 必须相符（簿记不变量）。
    let held = super::banker::BANKER.held_count();
    let outstanding = crate::memory::allocator::frame::outstanding();
    let held_ok = held == outstanding;
    if !held_ok {
        crate::putln!("[audit] banker held {held} != frame outstanding {outstanding}");
    }

    // 收尾：释放审计期收集存储（Audit 类豁免成对——unmark 不计数、drain 守卫
    // 保池页不归还），随后关审计期标记。report 路径（panic 现场）不依赖标记。
    drop(now_tables);
    drop(now_pool);
    super::AUDITING.store(false, Ordering::Relaxed);

    // 违例汇总（report panic 前全部诊断已落屏）。
    if task_frames > 0 || task_blocks > 0 {
        report(
            IntegrityViolation::AuditDivergence,
            0,
            format_args!(
                "task lifecycle leak at shutdown: {task_frames} frames, {task_blocks} blocks"
            ),
        );
    }
    if freed_persistent > 0 {
        report(
            IntegrityViolation::AuditDivergence,
            0,
            format_args!("{freed_persistent} persistent frames freed at shutdown"),
        );
    }
    if table_frames != walk_tables {
        report(
            IntegrityViolation::AuditDivergence,
            0,
            format_args!("table frames {table_frames} != kernel-walk {walk_tables} at shutdown"),
        );
    }
    if !held_ok {
        report(
            IntegrityViolation::AuditDivergence,
            0,
            format_args!("banker held {held} != frame outstanding {outstanding} at shutdown"),
        );
    }
    crate::putln!(
        "[audit] shutdown checks ok: task {task_frames}F/{task_blocks}B persistent-freed {freed_persistent} tables {table_frames}/{walk_tables} held {held}/{outstanding}"
    );
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

/// 全量审计（boot 收尾调用一次；三源交叉核对 + 类别计数 sanity，违例即 report）：
///   Banker.held_count == frame.outstanding；
///   帧类别计数之和 == held（每帧分配恰记入一个类别计数；Audit 类 boot 期无分配）；
///   块类别计数之和 == ledger 在册数（同帧侧；Audit 类 boot 期无登记）；
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
    // 类别计数 sanity：分配事件与类别记账一一对应（Audit 类跳过——boot 期无
    // 审计分配），类别计数之和必须等于 held / ledger 在册数。
    let ftotal: usize = super::FRAME_COUNTS
        .iter()
        .map(|c| c.load(Ordering::Relaxed))
        .sum();
    if ftotal != held {
        report(
            IntegrityViolation::AuditDivergence,
            0,
            format_args!("frame class counts {ftotal} != banker held {held}"),
        );
    }
    let btotal: usize = super::BLOCK_COUNTS
        .iter()
        .map(|c| c.load(Ordering::Relaxed))
        .sum();
    let recs = super::ledger::LEDGER.len();
    if btotal != recs {
        report(
            IntegrityViolation::AuditDivergence,
            0,
            format_args!("block class counts {btotal} != ledger records {recs}"),
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
