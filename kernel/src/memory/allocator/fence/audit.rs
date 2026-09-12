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

// ── 泄漏现场取证（只在日志一侧、只在已判泄漏时跑）────────

/// **谁还指着这个块**：把 `addr`（`ArcInner` 基址）当作字，扫一遍 DRAM，报出命中处。
///
/// # 为什么要有这一条
///
/// `leak: task 1` 的读数形态是 `strong 0 weak 1` —— 载荷已析构、块却因**一枚存活的
/// `Weak<Task>`** 而未归还。把全仓的 `Weak<Task>` 容器逐个点名排除（名册 / 票根 /
/// 躯壳 / 站点由 `rip` 清空；`Team.tasks` / `Team.sire` 由逐团队普查给出 0）之后，
/// 仍然复现 —— 说明**持有者不在我列举出来的容器里**。继续读代码只会继续漏，
/// 故改成**直接问内存**：`Weak` 对象里就存着这个基址，扫一遍必然命中持有它的那张表。
///
/// 代价：一次 DRAM 线性读（128 M ⇒ 千万级字），**只在已判泄漏时跑**（一次性诊断，
/// 机器已经坏了，速度不重要）。只扫内核镜像 + DRAM 窗口，不碰 MMIO。
///
/// 返回命中数；逐条打印命中地址（在内核镜像里的可直接 `nm` 符号化 = 哪一个静态）。
fn who_holds(addr: usize, cap: usize) -> usize {
    // **只扫"内核真的映射了"的两段**：内核镜像（静态/BSS）与帧池窗口**去掉保留区**
    // （initrd / 设备树）。越界即 page fault —— 本轮真踩过一次（扫到 `0x87f90000`），
    // 于是"诊断"把一次可读的泄漏报告升级成了 panic。
    let (pool_base, pool_edge, holes) = crate::memory::allocator::frame::heap().window();
    let mut hits = 0usize;
    let mut scan = |lo: usize, hi: usize, hits: &mut usize| {
        let mut p = (lo + 7) & !7;
        while p + 8 <= hi {
            // 跳过保留区（未映射/只读之外的区域）。
            if holes
                .iter()
                .flatten()
                .any(|&(s, e)| p >= pool_base + s * crate::memory::PAGE_SIZE && p < pool_base + e * crate::memory::PAGE_SIZE)
            {
                p += crate::memory::PAGE_SIZE;
                continue;
            }
            // SAFETY: 两段窗口都已由 boot 建立恒等映射，对齐读。
            let w = unsafe { *(p as *const usize) };
            if w == addr {
                *hits += 1;
                if *hits <= cap {
                    let zone = if p >= super::image_base() && p < super::image_edge() {
                        "内核镜像（静态/BSS ⇒ 可 nm 符号化）"
                    } else {
                        "帧池（堆/栈 ⇒ 动态分配）"
                    };
                    crate::putln!("[audit]     <- 指向它的字 @ {p:#x}（{zone}）");
                }
            }
            p += 8;
        }
    };
    scan(super::image_base(), super::image_edge(), &mut hits);
    scan(pool_base, pool_edge, &mut hits);
    hits
}

/// 打印该种类的**全部在册记录**：地址 / 尺寸 / 分配点 `site`（host `addr2line`
/// 可符号化）。`End::Zero` 判据只说"哪种对象没归零"，说不出"是哪一支分配点、
/// 哪个对象"——本条补上那两个量。**只读账本、只打印**（不改判据、不写任何表）。
///
/// `Kind::Task` 的记录是 `ArcInner<{Task, TaskIdent}>`（两种都由
/// `tagged_alloc(Kind::Task)` 标注，尺寸不同故可分辨），头 16 字节是
/// strong/weak 计数、+16 起是载荷——`Task` 的载荷首字是指向 `TaskIdent` 的
/// `Arc` 数据指针，`TaskIdent` 的载荷首字是 `id`、次字是 `name`（`&'static str`
/// 二元组 = 指针 + 长度）。故这里能**直接打出泄漏任务的名字**：名册只说"还有
/// 条目活着"，本条说得出"是哪一个"。
///
/// **锁纪律**：本条要两次问账本（先抄记录、再按地址指认内层 `TaskIdent`），而
/// `LEDGER.for_each` 是**持有** Ledger 锁的遍历——嵌套调用即自锁死（`ledger::retire`
/// 的头注已记过同一个坑）。故先把记录抄进**预先分配好**的缓冲（锁内零分配），
/// 放锁后再互相指认。
fn dump_records(kind: Kind) {
    use crate::work::unit::task::{Task, TaskIdent};

    /// 一条抄出来的记录（放锁后用它打印与互相指认）。
    struct Rec {
        addr: usize,
        size: usize,
        site: usize,
    }

    let mut recs: alloc::vec::Vec<Rec, &'static dyn Allocator> = alloc::vec::Vec::with_capacity_in(
        super::ledger::LEDGER.len().max(64),
        crate::memory::allocator::hybrid::allocator(),
    );
    super::ledger::LEDGER.for_each(|addr, rec| {
        if rec.kind == kind {
            recs.push(Rec {
                addr,
                size: rec.size,
                site: rec.site,
            });
        }
    });

    let task_bytes = core::mem::size_of::<Task>();
    let ident_bytes = core::mem::size_of::<TaskIdent>();
    let id_off = core::mem::offset_of!(TaskIdent, id);
    let name_off = core::mem::offset_of!(TaskIdent, name);
    let lo = super::image_base();
    let hi = super::image_edge();
    // 上限：取证输出不许自己变成刷屏源（同类事故已实证一次：58 MB 控制台）。
    // 截断只影响"打印多少"，报告头一行与判据都不动。
    const CAP: usize = 64;
    if recs.len() > CAP {
        crate::putln!("[audit]   ... {} records, first {CAP}:", recs.len());
    }
    for r in recs.iter().take(CAP) {
        crate::putln!(
            "[audit]   {} @ {:#x} size {} site {:#x}",
            kind.name(),
            r.addr,
            r.size,
            r.site
        );
        if kind != Kind::Task {
            continue;
        }
        // 块 = `ArcInner<T>`：头 16 字节是 strong/weak 计数，data = 基址 + 16。
        // SAFETY: 记录在册 = 块活、不可复用；下表读是只读诊断（与
        // `sweep_canaries` 同性质）。尺寸（块分配器报的**请求字节数**）分出是哪一种
        // `T`：`ArcInner<Task>` = 88、`ArcInner<TaskIdent>` = 48。
        let data = r.addr + 16;
        let size_class = r.size.max(8).next_power_of_two();
        unsafe {
            // `ArcInner<T> = { strong, weak, data }` ⇒ **strong 在 +0、weak 在 +8**。
            // 先前这里只读 +8 却标成 `strong`，于是"泄漏的都是 strong=1"这个读数
            // 是**错的**：那是 `weak`。两者含义天差地别 ——
            //   · `strong ≥ 1` = 真有强持有者（对象还活着，真泄漏）；
            //   · `strong == 0 && weak == 1` = 只有 `Weak` 活着 ⇒ 载荷已析构、块却
            //     因弱引用而**未归还**（站点墓碑/注册表里的一枚 `Weak<Task>` 即可
            //     造成），泄漏的是 152 B 的 `ArcInner` 外壳，不是活任务。
            // 故两个都读、都打（只读诊断，不改判据）。
            let strong = *(r.addr as *const u64);
            let weak = *((r.addr + 8) as *const u64);
            let word0 = *((data + id_off) as *const usize);
            let word1 = *((data + name_off) as *const usize);
            if size_class >= 16 + task_bytes + 8 {
                // 载荷首字 = 内层 `Arc<TaskIdent>` 的数据指针，按地址在本批记录里认它。
                let owner = recs.iter().find(|o| {
                    o.addr + 16 == word0 && o.size.max(8).next_power_of_two() < size_class
                });
                match owner {
                    Some(o) => crate::putln!(
                        "[audit]     <- Task block: strong {strong} weak {weak}, ident @ {word0:#x} (rec {:#x}, id {})",
                        o.addr,
                        *((word0 + id_off) as *const usize)
                    ),
                    None => crate::putln!(
                        "[audit]     <- Task block: strong {strong} weak {weak}, ident @ {word0:#x} (record not in this batch)"
                    ),
                }
            } else if size_class >= 16 + ident_bytes {
                crate::putln!("[audit]     <- TaskIdent: strong {strong} weak {weak}, id {word0}");
                // `name` = (&'static str)（指针 + 长度）：指针落在内核镜像内才读
                // （.rodata 的字面量；镜像恒等映射，S 态可直读）。
                if word1 >= lo && word1 < hi && word0 < 256 && word1 + word0 <= hi {
                    let s = core::slice::from_raw_parts(word1 as *const u8, word0);
                    if let Ok(s) = core::str::from_utf8(s) {
                        crate::putln!("[audit]     name \"{s}\"");
                    }
                }
            }
            // **谁还指着它**：载荷已析构（`strong 0`）而块未归还 ⇒ 有一枚弱引用活着。
            // 逐个容器点名排除之后仍然复现，故直接扫内存把持有者找出来（见 `who_holds`）。
            if strong == 0 {
                let hits = who_holds(r.addr, 8);
                crate::putln!("[audit]     -> 全 DRAM 命中 {hits} 处（上限打印 8）");
            }
        }
    }
}

/// **`rip` 之后的弱引用普查**（只读；`check_baseline` **之前**跑，与泄漏判据同一快照）。
///
/// # 为什么需要它（`leak: task 1` 的定案仪）
///
/// `ArcInner<Task>`（`Kind::Task`，152 B）的最后一门是**弱引用**：只要还有一枚
/// `Weak<Task>` 活着，块就归还不掉 —— 即使载荷早已析构。`scheduler::core::rip`
/// 把名册（全世界任务的弱引用）**放最后**清空，靠"最后一次弱引用归零 → 级联 drop"
/// 把外壳全带走。但**名册不是唯一的弱引用容器**：
///
/// ```text
/// 名册 ROSTER（rip 清空）        holders / sites / husks（rip 清空）
/// Team.tasks: Vec<Weak<Task>>    ← rip **不碰**（随团队析构才消失）
/// Team.sire:  Weak<Task>         ← rip **不碰**
/// ```
///
/// 全仓的 `Arc<Team>` 只有三处：`Task.heir`、`TaskIdent.team`、以及
/// `KERNEL_TEAM`（`OnceLock<Arc<Team>>`，**永不析构**）。前两者随任务载荷析构而消失，
/// 所以"停机时还活着的团队"只可能是内核团队 ⇒ 它那张成员表里的**死条目**是唯一
/// 能跨过 `rip` 的弱引用。本行把这件事变成两个数：`tasks 总/死` 与 `sire/held` 是否
/// 还活着（三者都应为 `0`/`false`），并顺带打出泄漏判据真正读的那个数（活着的
/// `Kind::Task` 块数）。
#[cfg(feature = "audit")]
pub fn probe_teams() {
    let live_tasks = crate::memory::allocator::statistics::view_block().kinds[Kind::Task as usize];
    let (live_teams, more, dead_total, head) = crate::work::unit::team::weak_census_all();
    let kernel_line = match crate::work::unit::team::kernel() {
        Some(t) => {
            let (n, dead) = t.weak_census();
            alloc::format!(
                "内核团队 tasks {n}（死 {dead}）sire_live={} held_live={}",
                t.sire_live(),
                t.held_live()
            )
        }
        None => alloc::string::String::from("内核团队未注入"),
    };
    crate::putln!(
        "[audit] teams 活 {live_teams}（超限 {more}）｜死弱引用 {dead_total}（首 4 个 (id, tasks 死, sire 死)={head:?}）\
         ｜{kernel_line}｜task 块存活={live_tasks}"
    );
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
            dump_records(k);
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
    // **容量必须留余量，且闭包里绝不许扩容**（实测踩过，release 档是静默死机）：
    // `with_capacity_in` 自己会向 hybrid 要一块内存，而块分配**要记账**
    // （`on_alloc` → `ledger::mark`）⇒ 它在这次预留**之后**给账本添一条记录。按
    // `len()` 预留就短一条，闭包里的 `push` 于是扩容——扩容再向块分配器要块、再记账，
    // 而 `mark` 要取的就是闭包正持有的那把 Ledger 锁：**8→8 自锁死**。
    // 现象：`audit()` 卡死在 `for_each` 里，没有任何输出（本仓实测：容量 65、账本 66 条）。
    // 双保险 = ① 多留 `SLACK` 条；② 闭包按 `capacity()` 判满，**永不扩容**，真放不下
    // 也只丢计数并**报出来**（不静默截断）。
    const SLACK: usize = 8;
    let cap = super::ledger::LEDGER.len() + SLACK;
    let mut kheap: alloc::vec::Vec<usize, &'static dyn Allocator> =
        alloc::vec::Vec::with_capacity_in(cap, crate::memory::allocator::hybrid::allocator());
    let mut dropped = 0usize;
    super::ledger::LEDGER.for_each(|addr, rec| {
        if rec.kind.poison() {
            if kheap.len() < kheap.capacity() {
                kheap.push(addr);
            } else {
                dropped += 1;
            }
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
    if dropped > 0 {
        report(
            IntegrityViolation::AuditDivergence,
            0,
            format_args!("{dropped} kernel-heap records beyond the audit buffer"),
        );
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
