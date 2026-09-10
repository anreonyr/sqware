//! 护栏层 · audit — 核查侧：所有权类别记账、关机不变量、页清残留检查
//!
//! 与 ledger（簿记：写账/读账）相对，本模块是**核查**：拿种类计数对
//! 不变量。只读账本（ledger/statistics 计数）与帧分配器的 `pagemeta` 读侧，不写。违例统一经
//! `report` 处置（见 fence/mod）。
//!
//! # 设计：逐对象种类记账（替代旧「boot 身份快照 vs 关机差集」，也替代粗粒度的 4 类）
//!
//! 每帧/每块按**对象种类**归属（[`super::Kind`]），计数由 statistics 维护
//! （statistics::view_frame/block().kinds 是唯一权威）；装饰器 `tag!` 在分配点标注、
//! 释放路径摘标。合法形态演化——容器扩容、realloc 搬家（种类继承）、池页周转、
//! 审计工具自身分配——只是种类内部的变化，**不需要任何赦免机制**。
//!
//! 关机检查 [`check_baseline`] 按**期望终值**（[`super::End`]）分组：
//!   ① `End::Zero` 逐种类归零——真泄漏判据，**报告点名是哪种对象**（替代旧「Task 帧 N」）。
//!   ② `End::Held` 持久注册表逐项仍 held           持久错还。
//!   ③ `End::Walk` 表页数 == 内核根表 walk 数      任务表遗留 / 内核表被摘。
//!   ④ `End::Report` 只报数（池借页周转 / 自检帧 / 未标注 Plain）。
//!   ⑤ `End::Retire` 随空间作废（用户堆账，`fence::retire` 已在归还 ASID 前销账）。
//!   ⑥ 帧侧"在不在手"只问 `frame::is_held`（pagemeta 唯一真相；banker 已删——
//!      原先那条 `banker held == frame.occupied` 是两份账记同一件事，随之消失）。
//!
//! boot 收尾 [`audit()`] 三源交叉核对 + 类别计数 sanity；[`page_clear`] 验页内
//! 无活账。

#![cfg(feature = "audit")] // audit feature（debug 默认开；release 可显式 --features audit）

use core::alloc::Allocator;
use core::sync::atomic::Ordering;

use crate::lock::OnceLock;
use crate::memory::manager::addr::PhysAddr;

use super::{End, IntegrityViolation, Kind, Side, report};
use crate::memory::allocator::statistics;

// ── 持久注册表 ────────────────────────────────────────

/// boot 持久帧登记：显式 **add-only** 注册 `(pa, name)`——trap 栈块、spare 仓块、
/// 内核窗口帧（各自 boot 初始化点注册）。硬规则：持久帧永不移动（移动的是缓冲，
/// 缓冲是块/池页）——注册表永远有效，无 rehome/adopt。关机逐项校验仍 held
/// （[`crate::memory::allocator::frame::FrameAllocator::is_held`]），错还即违例
/// （替代旧「持久缺失差集」——旧差集把
/// boot held 全集当持久集，任何合法归还都需赦免；注册表只覆盖**声明持久**的
/// 少量结构，其余 boot 期分配属默认类 Persistent，不参与检查）。
struct PersistEntry {
    pa: usize,
    /// 声明它是哪种对象（报表用名由种类给出——旧版是一份并列的字符串，
    /// 与种类重复且无人核对）。
    kind: Kind,
}

static PERSISTENT: OnceLock<crate::lock::SpinLock<alloc::vec::Vec<PersistEntry>>> = OnceLock::new();

/// 登记持久帧（boot 调用；add-only）。`pa` = 帧块基址（分配事件首地址——帧种类表
/// 与 held 核验均按分配事件首页）；`kind` = 声明它是哪种对象（名字从种类来）。
pub(crate) fn register_persistent(pa: usize, kind: Kind) {
    let list = PERSISTENT.get_or_init(|| crate::lock::SpinLock::new(alloc::vec::Vec::new()));
    list.lock().push(PersistEntry { pa, kind });
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
        (0x8000_0000..crate::machine::dram_edge().unwrap_or(0x9000_0000)).contains(&pa.as_usize())
    };
    if ok(PhysAddr::from_raw(root)) {
        descend(root, crate::memory::manager::mode::levels(), &ok, out);
    }
}

// ── 关机检查 ──────────────────────────────────────────

/// 帧"在不在手"的唯一问法（pagemeta 读侧；banker 删除后不再有第二份每页位图）。
fn held(pa: usize) -> bool {
    crate::memory::allocator::frame::heap().is_held(pa)
}

/// 按种类取「帧 + 块」两侧的计数（种类自带侧面；未标注两侧都算）。
fn count_kind(frame: &statistics::FrameView, block: &statistics::BlockView, k: Kind) -> usize {
    match k.side() {
        Some(Side::Frame) => frame.kinds[k as usize],
        Some(Side::Ledger) => block.kinds[k as usize],
        None => frame.kinds[k as usize] + block.kinds[k as usize],
    }
}

/// 断言关机时各对象种类都到了它的期望终值（逐种类，见模块头）。
///
/// 与旧框架（boot 快照差集）不同：本检查对「合法形态演化」天然免疫——容器
/// 扩容、realloc 搬家（新种类继承）、池页周转、审计工具自身分配都只是种类
/// 内部的变化，不构成违规、不需要赦免。
#[track_caller]
pub fn check_baseline() {
    let frame = statistics::view_frame();
    let block = statistics::view_block();

    // 收集存储物化：普通记账（Plain 类）——审计暂态分配与归还在本
    // 函数内成对，新框架无差集检查对其天然免疫。容量 ≥ 全集（held_count 上界），
    // push 零分配、无 realloc（关机单核无并发分配）。
    let mut now_tables: alloc::vec::Vec<usize, &'static dyn Allocator> =
        alloc::vec::Vec::with_capacity_in(
            frame.occupied.max(64),
            crate::memory::allocator::hybrid::allocator(),
        );
    let mut now_pool: alloc::vec::Vec<usize, &'static dyn Allocator> =
        alloc::vec::Vec::with_capacity_in(
            frame.occupied.max(64),
            crate::memory::allocator::hybrid::allocator(),
        );

    // ① 真泄漏：`End::Zero` 的**每一种**都必须归零，报告点名是哪种对象。
    let mut zero_total = 0usize;
    let mut zero_ok = 0usize;
    let mut leaks = 0usize;
    for k in Kind::ALL {
        if k.end() != End::Zero {
            continue;
        }
        zero_total += 1;
        let n = count_kind(frame, block, k);
        if n == 0 {
            zero_ok += 1;
        } else {
            crate::putln!("[audit] leak: {} {n}", k.name());
            leaks += 1;
        }
    }

    // ② 持久注册表：逐项仍 held（持久帧错还 = 违例）。
    let mut freed_persistent = 0usize;
    let mut held_total = 0usize;
    let mut misdeclared = 0usize;
    if let Some(reg) = PERSISTENT.get() {
        let g = reg.lock();
        for e in g.iter() {
            held_total += 1;
            // 声明即受核：登记说的种类必须与帧种类表里的一致——两处都在同一条
            // boot 语句上，一致是廉价的，不一致就是把种类说错了（旧版这里是一份
            // 并列的字符串，无人核对）。
            let actual = super::frame_kind(e.pa);
            if actual != e.kind && held(e.pa) {
                crate::putln!(
                    "[audit] persistent {} @ {:#x} is tagged {} in the frame table",
                    e.kind.name(),
                    e.pa,
                    actual.name()
                );
                misdeclared += 1;
            }
            if !held(e.pa) {
                if freed_persistent == 0 {
                    crate::putln!("[audit] freed persistent frames at shutdown:");
                }
                crate::putln!(
                    "  freed[{freed_persistent}] = {} @ {:#x}",
                    e.kind.name(),
                    e.pa
                );
                freed_persistent += 1;
            }
        }
    }

    // ③ 表页计数 vs 内核根表 walk。
    collect_kernel_tables(&mut now_tables);
    let walk_tables = now_tables.len();
    let table_frames = frame.kinds[Kind::Table as usize];
    if table_frames != walk_tables {
        crate::putln!("[audit] table frames {table_frames} != kernel-walk count {walk_tables}:");
        for (i, &pa) in now_tables.iter().take(16).enumerate() {
            crate::putln!("  walk[{i}] = {pa:#x}");
        }
    }

    // ④ 只报数：周转（池借页）、自检帧、未标注 Plain、随空间退役的用户堆账。
    let prime = frame.kinds[Kind::Prime as usize];
    let probe = frame.kinds[Kind::Probe as usize];
    let plain = count_kind(frame, block, Kind::Plain);
    let user_heap = block.kinds[Kind::UserHeap as usize];
    crate::memory::allocator::block::heap().collect_owned(&mut now_pool);
    let pool_pages = now_pool.len();

    drop(now_tables);
    drop(now_pool);

    if leaks > 0 {
        report(
            IntegrityViolation::AuditDivergence,
            0,
            format_args!("{leaks} object kinds leaked at shutdown"),
        );
    }
    if freed_persistent > 0 {
        report(
            IntegrityViolation::AuditDivergence,
            0,
            format_args!("{freed_persistent} persistent frames freed at shutdown"),
        );
    }
    if misdeclared > 0 {
        report(
            IntegrityViolation::MisplacedKind,
            0,
            format_args!("{misdeclared} persistent frames misdeclared"),
        );
    }
    if table_frames != walk_tables {
        report(
            IntegrityViolation::AuditDivergence,
            0,
            format_args!("table frames {table_frames} != kernel-walk {walk_tables} at shutdown"),
        );
    }
    crate::putln!(
        "[audit] shutdown checks ok: zero {zero_ok}/{zero_total} held {}/{held_total} tables {table_frames}/{walk_tables} report-only prime {prime} probe {probe} plain {plain} user-heap {user_heap} pool {pool_pages}",
        held_total - freed_persistent
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
///   帧种类计数之和 == frame.occupied（每帧分配恰记入一个种类计数）；
///   块类别计数之和 == ledger 在册数；
///   每条内核堆记录的地址须落在某池持有页、且所在页仍在帧分配器手里
///   （用户堆记录不参与：键是页索引，不是地址）。
pub fn audit() {
    let frame = statistics::view_frame();
    let block = statistics::view_block();
    let occupied = frame.occupied;
    // 帧种类计数之和 == occupied（帧侧唯一账：pagemeta 的 occupied 镜像）。
    let ftotal: usize = frame.kinds.iter().sum();
    if ftotal != occupied {
        report(
            IntegrityViolation::AuditDivergence,
            0,
            format_args!("frame kind counts {ftotal} != frames occupied {occupied}"),
        );
    }
    let btotal: usize = block.kinds.iter().sum();
    let recs = super::ledger::LEDGER.len();
    if btotal != recs {
        report(
            IntegrityViolation::AuditDivergence,
            0,
            format_args!("block class counts {btotal} != ledger records {recs}"),
        );
    }
    // **锁序**：帧侧读侧（`held`）要取 Frame 锁（level 6），而本处若在 Ledger 锁
    // （level 8）内取就是"持高取低"⇒ lockdep 当场报违规。故先在 Ledger 锁内把地址
    // 抄进**预先分配好**的缓冲（锁内零分配），放锁后再问帧分配器。
    // 这条边是删 banker 之后新出现的：banker 是无锁原子位图，不问帧锁。
    let mut kheap: alloc::vec::Vec<usize, &'static dyn Allocator> =
        alloc::vec::Vec::with_capacity_in(
            super::ledger::LEDGER.len().max(64),
            crate::memory::allocator::hybrid::allocator(),
        );
    super::ledger::LEDGER.for_each(|addr, rec| {
        if rec.kind.poison() {
            kheap.push(addr);
        }
        // 用户堆记录**没有帧侧对应物**：它的键是 `(asid, 页索引)`，不是地址，帧
        // 分配器无从作答（它的性命由空间持有——`retire(asid)` 在归还 ASID 前销账，
        // 验收在 `Space` 侧）。旧代码在这里问 banker，那会撞上 `idx` 的范围断言
        // ——只因为关机前用户堆账已被 `retire` 清空，那个分支从未被走到（种类分开
        // 之后这里不再假装能查）。
    });
    for &addr in kheap.iter() {
        let page = addr & !(crate::memory::PAGE_SIZE - 1);
        if crate::memory::allocator::block::heap().own(addr).is_none() {
            report(
                IntegrityViolation::WildAddress,
                addr,
                format_args!("kernel-heap record outside block-owned pages"),
            );
        }
        if !held(page) {
            report(
                IntegrityViolation::AuditDivergence,
                addr,
                format_args!("kernel-heap record on non-held page {page:#x}"),
            );
        }
    }
    drop(kheap);

    // 收尾 delta：把与 boot 基线的差打印出来（statistics 的读侧出口）。
    if let Ok(d) = statistics::delta() {
        crate::putln!(
            "[audit] delta frame: total {:+} avail {:+} occ {:+}; block: occ {:+}; spare: total {:+} occ {:+} avail {:+}",
            d.frame.total,
            d.frame.available,
            d.frame.occupied,
            d.block.occupied,
            d.spare.total,
            d.spare.occupied,
            d.spare.available,
        );
    }
}

// ── 关机观测：messenger 簿记规模（只读）──────────────────

/// 打印 messenger 三张簿记表的规模——**全部任务退出时刻**的那一帧。
///
/// 为什么在这里：`prune`（空站点出队即删）每次收队都在跑却零观测量，站点表规模
/// 此前从哪都读不到，「删没删」只能靠读代码断言。三张表都只由关机钩子清，`rip`
/// 之后恒为 0 ⇒ 计数必须**排在 `rip` 之前**才有意义（这就是本函数被单列成一条
/// 钩子、而不并进 [`check_baseline`] 的理由）。
///
/// **只读**：不加 envcall、不改任何对外调用号，也不改任何调度/等待语义——
/// 量完即返回，由后续钩子照旧清表。
///
/// 判据（门侧断言的三项，按牙口从强到弱）：
///   ① `orphan == 0`：队列空、且（无信标 ∨ 键已死）的站点**只**该由 `prune` 删
///      ——这是对 `prune` 的直接断言；
///   ② `waiters == 0` 且 `live == 0`：全部任务已退出 ⇒ 不该还有人挂在任何站点上
///      （有 ⇒ 那张站点也在表里，直接与「全退出」矛盾）；
///   ③ `sites` / `tomb`：A2 落地后**恒为 0**（站点寿命＝资源寿命；`wipe` 不再留
///      墓碑、也不留空壳）。它们仍原样打印——一个计数被删掉就再也验不了「没有墓碑」，
///      而这正是 A2 的判据 1（34 → 0）。
pub fn probe_messenger() {
    let st = crate::work::room::messenger::probe();
    crate::putln!(
        "[audit] sites {} live {} tomb {} orphan {} dead {} waiters {}",
        st.sites,
        st.live,
        st.tomb,
        st.orphan,
        st.dead,
        st.waiters
    );
    crate::putln!("[audit] sites by kind: {}", st.kinds());
    let (holders_n, husks_n) = crate::work::room::messenger::probe_bookkeeping();
    crate::putln!("[audit] holders {holders_n} husks {husks_n}");
    // 名册：关机时不该还有能升起来的任务（强引用能升 ⇒ 有人没放 ⇒ 它钉住了自己的
    // Team/Space，帧与页因此留在类别账上——这就是 `task lifecycle leak` 那条账的名字）。
    let (roster_n, alive_n) = crate::work::room::scheduler::core::roster_live();
    crate::putln!("[audit] roster {roster_n} alive {alive_n}");
}
