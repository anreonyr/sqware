//! scene — 崩溃现场转储（定位错误的统一诊断）。
//!
//! 职责：回答「崩在哪个地址 / 哪条调用链」。读现场（GPR + 关键 CSR + 栈回溯）并
//! 格式化直写控制台；与 trace（结构化事件窗口）正交: trace 回答「崩前发生了什么」,
//! scene 回答「崩在哪」。
//!
//! 现场语义（诚实边界）：panic_handler 里实时读到的 GPR 是处理器已压栈损坏的
//! 现场；真正可定位的是 CSR 的 sepc/scause/stval（trap 进入后持续有效，直到下一次
//! trap）与栈回溯。GPR 尽力而为，sepc/stval 与回溯才是「where」。
//!
//! 回溯 = 无帧指针启发式：扫描当前栈区间，收集可执行地址（内核恒等/高半区或
//! 用户运行 team 表内）的候选返回地址（去重、深度封顶）。sepc/stval/回溯每条
//! 都经 elftable::resolve 符号化（命中出函数名），未命中打印裸 hex。
//!
//! 输出：标题（顶格）+ 段落（缩进表格），经 table::Para 渲染——一标题 = 一张表
//! （段落内多表列宽无法对齐，CSR+GPR 合并单表才是对齐正道）；行 = (label, value,
//! note) 三槽经 row3 双写：控制台表 + 宿主 scene 行（同一 &str 两个消费端）。
//! 无堆无锁；与 lock/depend 同段落格式。地址符号化经 elftable（含用户 team），
//! 详见 write_addr / addr_note。

use core::arch::asm;
use core::fmt::Write;

use alloc::sync::Arc;
use riscv::interrupt::{Exception, Interrupt, Trap};
use riscv::register::{satp, scause, sepc, sscratch, sstatus, stval, stvec};

use crate::console::Sink;
use crate::memory::PAGE_SIZE;
use crate::memory::manager::addr::VirtAddr;
use crate::runtime::switcher::context::{Gprs, TrapContext};
use crate::work::room::scheduler::{running_task_frame, running_task_info, running_team_try};
use crate::work::unit::elftable::{self, ElfTable};
use table::{Cell, Fmt, Para, RowsMut, Table, Width};

/// 回溯深度上限。
const BT_DEPTH: usize = 32;
/// 栈扫描窗口（从当前 sp 向上）字节数。
const BT_SCAN: usize = 4096;
/// 候选返回地址需 4 字节对齐（RISC-V 指令 2/4 字节）。
const ADDR_ALIGN: usize = 4;

// Sv39 PTE 位（诊断路径零依赖：直读 u64，不走 entry::Pte 类型）。
const PTE_V: u64 = 1 << 0;
const PTE_R: u64 = 1 << 1;
const PTE_W: u64 = 1 << 2;
const PTE_X: u64 = 1 << 3;
/// PTE.PPN 取位（bit 10 起 44 位 → 左移 12 得物理地址）。
const PTE_PPN_MASK: u64 = (1u64 << 44) - 1;

/// 定宽 hex（{:#018x}）——值列的通用形态（经 Fmt::hexw）。
fn hx(x: usize) -> Fmt<40> {
    let mut v = Fmt::<40>::new();
    v.hexw(x);
    v
}

/// 写地址（保留 user team 符号化）：命中出「函数+偏移」，否则裸 hex。
///
/// 注意：这点刻意不用 table::Fmt::addr——scene 需用户空间 team 符号化，而
/// Fmt::addr 走全局内核符号器（team=None）。这里写入传入的 sink，由调用方收行。
fn write_addr<W: Write>(w: &mut W, a: usize) {
    let va = VirtAddr::from_raw(a);
    if let Some((name, off)) = elftable::resolve(va, running_team_try().as_deref()) {
        let _ = write!(w, "{:#}", sym_name(name));
        let _ = write!(w, "+{off:#x}");
    } else {
        let _ = write!(w, "{a:#x}");
    }
}

/// 符号名可读化：经 rustc_demangle（v0/legacy 全语法，流式零分配——panic 现场
/// 不碰堆）；非 Rust 符号（内核表字面量 / no_mangle 名）原样透传，坏语法出
/// `{invalid syntax}` 占位（有界、不 panic）。
fn sym_name(name: &str) -> rustc_demangle::Demangle<'_> {
    rustc_demangle::demangle(name)
}

/// 行尾符号化注解：地址的定宽值已写，命中出符号则追加「name+off」（未命中无注解）。
/// 无前缀——列间分隔由 Table 的 cell padding 统一负责（避免与语义注解叠成多层空格）。
fn addr_note<W: Write>(w: &mut W, a: usize) {
    let va = VirtAddr::from_raw(a);
    if let Some((name, off)) = elftable::resolve(va, running_team_try().as_deref()) {
        let _ = write!(w, "{:#}", sym_name(name));
        let _ = write!(w, "+{off:#x}");
    }
}

/// 符号注解 → Fmt 缓冲（与值列同源，供 row3 双写）。
fn addr_note_v(a: usize) -> Fmt<96> {
    let mut n = Fmt::<96>::new();
    addr_note(&mut n, a);
    n
}

/// 一行 scene 记录 → 宿主导出文件（JSON `{"h","t","kind":"scene","tbl",
/// "label","v","n"}`；feature semihosting 时启用）。与控制台表格同源（同一
/// 局部 label/v/n），值/注解双写；不启用时 no-op——调用点无条件、表格照旧。
fn scene_row(tbl: &str, label: &str, v: &str, n: &str) {
    #[cfg(feature = "semihosting")]
    {
        use crate::runtime::diagnose::export::{k, line, v as jv};
        let h = crate::machine::hart_id();
        let t = crate::runtime::chrono::clock::now().as_ticks();
        line(|w| {
            let _ = write!(
                w,
                "\"h\":{h},\"t\":{t},\"kind\":\"scene\",\"tbl\":\"{tbl}\""
            );
            let _ = k(w, "label");
            let _ = jv(w, label);
            let _ = k(w, "v");
            let _ = jv(w, v);
            let _ = k(w, "n");
            let _ = jv(w, n);
        });
    }
    #[cfg(not(feature = "semihosting"))]
    {
        let _ = (tbl, label, v, n);
    }
}

/// 三槽行：label/value/note 入当前行，同值导出 scene 行（tbl 由调用方给，
/// 两消费端不可能只写一边）。行耗尽（上限满，调用方 bug）静默跳过，不 panic。
fn row3(it: &mut RowsMut<'_, 3, 96>, tbl: &str, label: &str, v: &str, n: &str) {
    let Some(row) = it.next() else {
        return;
    };
    row[0] = Cell::new(label);
    row[1] = Cell::new(v);
    row[2] = Cell::new(n);
    scene_row(tbl, label, v, n);
}

/// 两槽行：label/value 入表，note 恒空（GPR 无注解列）。
fn row2(it: &mut RowsMut<'_, 2, 96>, tbl: &str, label: &str, v: &str) {
    let Some(row) = it.next() else {
        return;
    };
    row[0] = Cell::new(label);
    row[1] = Cell::new(v);
    scene_row(tbl, label, v, "");
}

/// 读全部 31 个非零 GPR（x0 恒 0；ra/sp/gp/tp 首页）。
fn gprs() -> [usize; 32] {
    let mut r = [0usize; 32];
    unsafe {
        asm!("mv {0}, ra", out(reg) r[1]);
        asm!("mv {0}, sp", out(reg) r[2]);
        asm!("mv {0}, gp", out(reg) r[3]);
        asm!("mv {0}, tp", out(reg) r[4]);
        asm!("mv {0}, t0", out(reg) r[5]);
        asm!("mv {0}, t1", out(reg) r[6]);
        asm!("mv {0}, t2", out(reg) r[7]);
        asm!("mv {0}, s0", out(reg) r[8]);
        asm!("mv {0}, s1", out(reg) r[9]);
        asm!("mv {0}, a0", out(reg) r[10]);
        asm!("mv {0}, a1", out(reg) r[11]);
        asm!("mv {0}, a2", out(reg) r[12]);
        asm!("mv {0}, a3", out(reg) r[13]);
        asm!("mv {0}, a4", out(reg) r[14]);
        asm!("mv {0}, a5", out(reg) r[15]);
        asm!("mv {0}, a6", out(reg) r[16]);
        asm!("mv {0}, a7", out(reg) r[17]);
        asm!("mv {0}, s2", out(reg) r[18]);
        asm!("mv {0}, s3", out(reg) r[19]);
        asm!("mv {0}, s4", out(reg) r[20]);
        asm!("mv {0}, s5", out(reg) r[21]);
        asm!("mv {0}, s6", out(reg) r[22]);
        asm!("mv {0}, s7", out(reg) r[23]);
        asm!("mv {0}, s8", out(reg) r[24]);
        asm!("mv {0}, s9", out(reg) r[25]);
        asm!("mv {0}, s10", out(reg) r[26]);
        asm!("mv {0}, s11", out(reg) r[27]);
        asm!("mv {0}, t3", out(reg) r[28]);
        asm!("mv {0}, t4", out(reg) r[29]);
        asm!("mv {0}, t5", out(reg) r[30]);
        asm!("mv {0}, t6", out(reg) r[31]);
    }
    r
}

/// 栈回溯：扫描当前栈窗口，收集可执行候选返回地址到 out，返回帧数。
fn backtrace(out: &mut [usize; BT_DEPTH]) -> usize {
    let sp: usize;
    unsafe {
        asm!("mv {0}, sp", out(reg) sp);
    }
    let low = sp & !7usize;
    let high = sp.saturating_add(BT_SCAN);
    let mut n = 0usize;
    let mut prev = 0usize;
    let mut a = low;
    while a < high && n < BT_DEPTH {
        // SAFETY: 只读本线程栈区间（S 态直读恒等映射内存，无副作用）。
        let w = unsafe { (a as *const usize).read_volatile() };
        // 可执行候选 = 「能解析进符号区间」的地址：resolve 已按域（内核/用户）选表，
        // 并经 lookup 上界约束（下一符号/尾部跨度）。栈上的数据字（保存的 sp、撕裂
        // 值、ASCII 串）在此全部落空——不再用 is_kernel_addr 的宽镜像区间（含
        // .bss/.rodata）误收。
        let code =
            elftable::resolve(VirtAddr::from_raw(w), running_team_try().as_deref()).is_some();
        if w & (ADDR_ALIGN - 1) == 0 && w != prev && code {
            out[n] = w;
            n += 1;
            prev = w;
        }
        a += 8;
    }
    n
}

/// Sv39 页表 walk：把用户 **页对齐** VA 翻译为物理页（leaf 且带 R），未映射/无
/// 读权 → None。纯物理直读（恒等映射），零锁零 panic——诊断路径安全，不借
/// `Space::translate`（其持 RelLock，panic 现场可能死锁）。
///
/// **PA 范围守卫**：根表/中间表/leaf 的 PA 必须落在 DRAM 恒等区，否则直接返回
/// None——用户 satp 字段若被覆写为坏值，裸读会触发 S 态读 fault → 嵌套 panic
/// → halt 卡死（"panic 后不停机"的根路径）。
fn walk_sv39(page_va: usize, root_ppn: usize) -> Option<usize> {
    // DRAM 恒等区（覆盖 128M/256M 机器配置；QEMU virt DRAM 基址恒 0x80000000）。
    let in_dram = |pa: usize| (0x8000_0000..0x9000_0000).contains(&pa);
    let mut tbl = root_ppn << 12;
    if !in_dram(tbl) {
        return None;
    }
    for level in [2usize, 1, 0] {
        let idx = (page_va >> (12 + level * 9)) & 0x1ff;
        // SAFETY: tbl 已校验在 DRAM 内；诊断路径只读，其余核已冻结。
        let pte = unsafe { ((tbl + idx * 8) as *const u64).read_volatile() };
        if pte & PTE_V == 0 {
            return None;
        }
        if pte & (PTE_R | PTE_W | PTE_X) != 0 {
            // leaf 页：需可读（我们只读栈）且 PA 在 DRAM。
            if pte & PTE_R == 0 {
                return None;
            }
            let pa = (((pte >> 10) & PTE_PPN_MASK) << 12) as usize;
            return in_dram(pa).then_some(pa);
        }
        tbl = (((pte >> 10) & PTE_PPN_MASK) as usize) << 12; // 下一级表
        if !in_dram(tbl) {
            return None;
        }
    }
    None
}

/// 用户侧回溯：崩溃时 running 任务的 **用户 trap 帧**里有该任务最近一次用户态
/// 现场（SPP=User 时即被中断的当下；SPP=S 的内核态崩溃也保留着它上次进 trap
/// 的用户现场——用户程序正停在哪次 envcall 的答案）。取用户 sp 按用户页表逐页
/// 安全读栈窗口，收集可执行候选返回地址（判定 = **本任务自己的符号表**命中 +
/// 对齐 + 去重——不用跨核互踩的 `RUNNING_TEAM` 镜像，否则用户地址会查进别的
/// 团队的表而落空）。尽力而为：帧拿不到 / 现场非用户态（sepc 在内核域）/
/// sp 无效 / 栈页不可读 → 0 帧（不渲染 ubt 表）。
/// 返回 (帧数, 任务符号表 Arc)——渲染行仍用同一张表。
fn user_backtrace(out: &mut [usize; BT_DEPTH]) -> (usize, Option<Arc<ElfTable>>) {
    let Some((_, pa, tbl)) = running_task_frame() else {
        return (0, None);
    };
    // SAFETY: 帧 PA 在用户 Frame 窗口（DRAM，恒等映射）；崩溃现场读只读、其余核冻结。
    let frame = unsafe { &*(pa as *const TrapContext) };
    // 现场须属用户态：sepc 在内核域 ⇒ 该帧不含用户现场（从未跑用户/内核帧占位）→ 跳过。
    if frame.sepc.is_kernel() {
        return (0, tbl);
    }
    let (sp, root_ppn) = (frame.gpr.x(Gprs::SP), frame.user_satp.ppn());
    if sp == 0 {
        return (0, tbl);
    }
    let high = sp.saturating_add(BT_SCAN);
    let mut n = 0usize;
    let mut prev = 0usize;
    let mut page = sp & !(PAGE_SIZE - 1);
    while page < high && n < BT_DEPTH {
        let Some(pa0) = walk_sv39(page, root_ppn) else {
            page += PAGE_SIZE;
            continue;
        };
        let start = core::cmp::max(page, sp & !7);
        let end = core::cmp::min(page + PAGE_SIZE, high);
        let mut a = start;
        while a < end && n < BT_DEPTH {
            // SAFETY: 该页已 walk 命中且带 R；offset 恒在页内，S 态直读。
            let w = unsafe { ((pa0 + (a - page)) as *const usize).read_volatile() };
            // 用户域候选判定：能解析进本任务符号区间 + 4B 对齐 + 去重。
            let code = tbl
                .as_deref()
                .is_some_and(|t| t.lookup(VirtAddr::from_raw(w)).is_some());
            if w & (ADDR_ALIGN - 1) == 0 && w != prev && code {
                out[n] = w;
                n += 1;
                prev = w;
            }
            a += 8;
        }
        page += PAGE_SIZE;
    }
    (n, tbl)
}

/// stval 解码：按 scause 的语义注解（fault 地址 / 指令位 / 断点地址）；
/// 无有价值语义时输出 Unknown（中断 / ecall / 保留码 stval 均无定义）。
fn stval_note(int: bool, code: usize) -> &'static str {
    if int {
        return "Unknown";
    }
    match code {
        0 | 1 | 4 | 5 | 6 | 7 | 12 | 13 | 15 => "faulting addr",
        2 => "Illegal instruction bits",
        3 => "Breakpoint",
        _ => "Unknown",
    }
}

/// 表头行（三列表）：块名 + 列语义（hex/note）。纯渲染装饰，不导出 scene 记录；
/// 首列与内容行同宽（列宽统一），dump 内各表头风格一致。
fn header3(it: &mut RowsMut<'_, 3, 96>, name: &str) {
    let Some(row) = it.next() else {
        return;
    };
    row[0] = Cell::new(name);
    row[1] = Cell::new("hex");
    row[2] = Cell::new("note");
}

/// 表头行（两列表）：块名 + 列语义。
fn header2(it: &mut RowsMut<'_, 2, 96>, name: &str, col: &str) {
    let Some(row) = it.next() else {
        return;
    };
    row[0] = Cell::new(name);
    row[1] = Cell::new(col);
}

/// CSR 块填充（首行表头，其后至多 8 行）：sepc/stval/scause = 崩点；stvec/sscratch =
/// 陷阱入口/暂存；sstatus/satp = 特权/地址空间域。注解列 = 符号化（sepc/stval/
/// stvec）+ 解码（scause 用 riscv 枚举、stval 语义、sscratch 约定、sstatus 位域、
/// satp 用 Mode 枚举）。task 行 = 运行中任务（若有；try_lock 拿不到则跳过）。
fn fill_csrs(it: &mut RowsMut<'_, 3, 96>) {
    header3(it, "csr");
    let sc = scause::read();
    let (int, code) = (sc.is_interrupt(), sc.code());
    if let Some((tid, name)) = running_task_info() {
        let mut v = Fmt::<64>::new();
        let _ = write!(v, "#{tid} '{name}'");
        row3(it, "csr", "task", v.as_str(), "");
    }
    row3(
        it,
        "csr",
        "sepc",
        hx(sepc::read()).as_str(),
        addr_note_v(sepc::read()).as_str(),
    );
    {
        // 符号命中 → 「sym note」单空格衔接；未命中 → 仅 note（无前缀）。
        // 不写固定「  」前缀，避免与 addr_note 叠成多余空格。
        let a = stval::read();
        let mut n = Fmt::<96>::new();
        let va = VirtAddr::from_raw(a);
        if let Some((name, off)) = elftable::resolve(va, running_team_try().as_deref()) {
            let _ = write!(n, "{name}+{off:#x} ");
        }
        let _ = write!(n, "{}", stval_note(int, code));
        row3(it, "csr", "stval", hx(a).as_str(), n.as_str());
    }
    {
        // 类型化枚举（同 trap 分发）：Trap<Interrupt, Exception> Debug 即
        // 变体名（UserEnvCall / LoadPageFault / SupervisorTimer…），本身自解释，
        // 不加 int/exc 前缀（中断/异常由 bit63 隐含，hex 值列可查）。非法码回退
        // Unknown——崩溃现场不 panic；后续 CSR 行照常渲染。
        let mut n = Fmt::<96>::new();
        let trap: Option<Trap<Interrupt, Exception>> = sc.cause().try_into().ok();
        match trap {
            Some(Trap::Interrupt(i)) => {
                let _ = write!(n, "{:?}", i);
            }
            Some(Trap::Exception(e)) => {
                let _ = write!(n, "{:?}", e);
            }
            None => {
                let _ = write!(n, "Unknown");
            }
        }
        row3(it, "csr", "scause", hx(sc.bits()).as_str(), n.as_str());
    }
    row3(
        it,
        "csr",
        "stvec",
        hx(stvec::read().address()).as_str(),
        addr_note_v(stvec::read().address()).as_str(),
    );
    {
        // sscratch 语义（内核约定，见 trampoline）：0 = 内核态约定；非 0 =
        // 用户态陷阱入口/线程帧相关（该值 = 曾写进 sscratch 的用户 sp 或帧自址）。
        // 注解简化为特权归属：Kernel / User。崩溃多在内核态，0 即常态。
        let scr = sscratch::read();
        let n = if scr == 0 { "Kernel" } else { "User" };
        row3(it, "csr", "sscratch", hx(scr).as_str(), n);
    }
    {
        let ss = sstatus::read();
        // 注解只列非默认态：前特权模式（User/Supervisor）恒打——崩溃在用户/
        // 内核态的定位关键；布尔位置位才打缩写（SIE/SPIE/SUM/MXR/SD）；
        // FS/VS/XS 非 Off 才打短码（Off=未启用省略；短码表见各分支注释）。
        let mut note = Fmt::<96>::new();
        let _ = write!(note, "{:?}", ss.spp());
        if ss.sie() {
            let _ = write!(note, " SIE");
        }
        if ss.spie() {
            let _ = write!(note, " SPIE");
        }
        if ss.fs() != riscv::register::mstatus::FS::Off {
            // FS 短码：Initial→FI / Clean→FC / Dirty→FD（Off 为未启用，不打印）。
            // 保守 else：不 panic（诊断路径零 panic），Off 分支实际不会进入。
            let code = match ss.fs() {
                riscv::register::mstatus::FS::Initial => "FI",
                riscv::register::mstatus::FS::Clean => "FC",
                riscv::register::mstatus::FS::Dirty => "FD",
                riscv::register::mstatus::FS::Off => "FO",
            };
            let _ = write!(note, " FS {code}");
        }
        if ss.vs() != riscv::register::mstatus::VS::Off {
            // VS 短码：Initial→VI / Clean→VC / Dirty→VD（结构同 FS）。
            let code = match ss.vs() {
                riscv::register::mstatus::VS::Initial => "VI",
                riscv::register::mstatus::VS::Clean => "VC",
                riscv::register::mstatus::VS::Dirty => "VD",
                riscv::register::mstatus::VS::Off => "VO",
            };
            let _ = write!(note, " VS {code}");
        }
        if ss.xs() != riscv::register::mstatus::XS::AllOff {
            // XS 短码（结构同 FS/VS，语义平行：NoneDirtyOrClean=Initial 等价）：
            // NoneDirtyOrClean→XI / NoneDirtySomeClean→XC / SomeDirty→XD；
            // AllOff=未启用，不打印。
            let code = match ss.xs() {
                riscv::register::mstatus::XS::NoneDirtyOrClean => "XI",
                riscv::register::mstatus::XS::NoneDirtySomeClean => "XC",
                riscv::register::mstatus::XS::SomeDirty => "XD",
                riscv::register::mstatus::XS::AllOff => "XA",
            };
            let _ = write!(note, " XS {code}");
        }
        if ss.sum() {
            let _ = write!(note, " SUM");
        }
        if ss.mxr() {
            let _ = write!(note, " MXR");
        }
        if ss.sd() {
            let _ = write!(note, " SD");
        }
        row3(it, "csr", "sstatus", hx(ss.bits()).as_str(), note.as_str());
    }
    {
        let s = satp::read();
        let mut n = Fmt::<96>::new();
        let _ = write!(n, "{:?} {:#06x} {:#013x}", s.mode(), s.asid(), s.ppn(),);
        row3(it, "csr", "satp", hx(s.bits()).as_str(), n.as_str());
    }
}

/// GPR 块填充（首行表头，其后只打非零）：label/hex 两槽，note 空。
fn fill_gprs(it: &mut RowsMut<'_, 2, 96>) {
    header2(it, "gpr", "hex");
    const NAMES: [&str; 32] = [
        "x0", "ra", "sp", "gp", "tp", "t0", "t1", "t2", "s0", "s1", "a0", "a1", "a2", "a3", "a4",
        "a5", "a6", "a7", "s2", "s3", "s4", "s5", "s6", "s7", "s8", "s9", "s10", "s11", "t3", "t4",
        "t5", "t6",
    ];
    let r = gprs();
    for (i, name) in NAMES.iter().enumerate().skip(1) {
        if r[i] == 0 {
            continue;
        }
        row2(it, "gpr", name, hx(r[i]).as_str());
    }
}

/// 回溯两槽行：#i / 符号化地址；导出行 v = 定宽 hex、n = 符号。
/// tbl 传 "kbt"（内核栈）/ "ubt"（用户栈），scene 记录按表区分。
fn bt_row(it: &mut RowsMut<'_, 2, 96>, tbl: &str, i: usize, a: usize) {
    let Some(row) = it.next() else {
        return;
    };
    let mut l = Fmt::<16>::new();
    let _ = write!(l, "#{i}");
    let mut s = Fmt::<96>::new();
    write_addr(&mut s, a);
    row[0] = Cell::new(l.as_str());
    row[1] = Cell::new(s.as_str());
    scene_row(tbl, l.as_str(), hx(a).as_str(), s.as_str());
}

/// ubt 两槽行：**按任务自己的符号表**（与 `user_backtrace` 收集同表——不依赖
/// 报警核 `running_team_try` 的镜像）符号化；命中出 name+off，未命中裸 hex。
/// name 经 `lookup` 的 name_in_range 防御（坏条目降级裸 hex，check_integrity
/// 现场自报），panic 现场打印安全（CSR/sepc 注解同路径已实证）。
fn ubt_row(it: &mut RowsMut<'_, 2, 96>, i: usize, a: usize, tbl: Option<Arc<ElfTable>>) {
    let Some(row) = it.next() else {
        return;
    };
    let mut l = Fmt::<16>::new();
    let _ = write!(l, "#{i}");
    let mut s = Fmt::<96>::new();
    if let Some((name, off)) = tbl
        .as_deref()
        .and_then(|t| t.lookup(VirtAddr::from_raw(a)))
    {
        let _ = write!(s, "{:#}", sym_name(name));
        let _ = write!(s, "+{off:#x}");
    } else {
        let _ = write!(s, "{a:#x}");
    }
    row[0] = Cell::new(l.as_str());
    row[1] = Cell::new(s.as_str());
    scene_row("ubt", l.as_str(), hx(a).as_str(), s.as_str());
}

/// 统一崩溃现场转储：一个 [scene] 标题统辖全部表——CSR 三列表 + GPR 两列表 +
/// 回溯表（内核栈 kbt + 用户栈 ubt，表头分块 + 空行）。u-thread 被中断上下文为
/// 用户态时 bt 后附 ubt（从 trap 帧用户 sp 经页表 walk 安全读栈，见
/// `user_backtrace`；读不到即 0 帧不渲染）。末尾倒出每 hart 最近事件窗口。
pub fn dump_crash() {
    // ① 探针：panic 现场 drop-in 完整性体检——越界写破坏用户符号表/相邻活块
    // 时自报（lookup 已降级裸 hex，体检负责把「破坏存在」说出来，含首例地址 /
    // 受害块分配点）。运行团队表体检 debug 才编译；ledger canary 清查额外需
    // audit feature。两者均纯读零分配、只经 putln! 直写 SBI 控制台——panic
    // 现场（含持锁）安全，且不截断本次转储。
    #[cfg(debug_assertions)]
    {
        if let Some(t) = running_team_try().as_deref() {
            if let Some(et) = t.elftable.as_ref() {
                et.check_integrity();
            }
        }
        #[cfg(feature = "audit")]
        {
            let _ = crate::memory::allocator::fence::ledger::LEDGER.sweep_canaries();
        }
    }
    // [scene] 标题 + CSR 表 + GPR 表 + 回溯表（表间空行由 Para::table 统一）。
    let mut p = Para::new(Sink);
    p.title(format_args!(
        "[scene] crash scene, hart {}",
        crate::machine::hart_id()
    ));
    let mut csr = Table::<3, 9, 96>::new();
    csr.set_width(0, Width::fixed(10));
    csr.set_total_width(64); // 与 [trace] 表同宽（统一诊断表宽预算）
    {
        let mut it = csr.rows_mut();
        fill_csrs(&mut it);
    }
    p.table(&csr);
    let mut gpr = Table::<2, 32, 96>::new();
    gpr.set_width(0, Width::fixed(10));
    gpr.set_total_width(64); // 三表同宽（末列截断上限）。
    {
        let mut it = gpr.rows_mut();
        fill_gprs(&mut it);
    }
    p.table(&gpr);
    let mut bt = [0usize; BT_DEPTH];
    let n = backtrace(&mut bt);
    let mut b = Table::<2, { BT_DEPTH + 1 }, 96>::new();
    b.set_width(0, Width::fixed(10));
    b.set_total_width(64); // 统一诊断表宽预算（同 csr）。
    {
        let mut it = b.rows_mut();
        header2(&mut it, "kbt", "sym");
        for (i, a) in bt[..n].iter().enumerate() {
            bt_row(&mut it, "kbt", i, *a);
        }
    }
    p.table(&b);

    // 用户侧回溯（kbt = 内核栈；ubt = 用户栈）——被中断上下文为用户态、能取到
    // running 任务帧时附加；0 帧不渲染（尽力而为，失败不 panic）。渲染色沿用
    // 任务自己的符号表（user_backtrace 带回），不依赖全局镜像。
    let mut ubt = [0usize; BT_DEPTH];
    let (m, tbl_arc) = user_backtrace(&mut ubt);
    if m > 0 {
        // ubt 表：符号化用任务自己的表（收集同表）；命中出 name+off，未命中
        // 裸 hex（lookup 的 name_in_range 防御下打印安全；表损坏由体检现场
        // 自报、行降级裸 hex——不再是「干脆全裸 hex」的旧裁量）。
        let mut u = Table::<2, { BT_DEPTH + 1 }, 96>::new();
        u.set_width(0, Width::fixed(10));
        u.set_total_width(64);
        {
            let mut it = u.rows_mut();
            header2(&mut it, "ubt", "sym");
            for (i, a) in ubt[..m].iter().enumerate() {
                ubt_row(&mut it, i, *a, tbl_arc.clone());
            }
        }
        p.table(&u);
    }
    crate::putln!(); // 段尾空行（块间间距统一；供 [trace] 段分隔）。

    // 每 hart 最近事件窗口文本统一倒出（报警核；崩溃后其余核已停写）。
    // JSON 侧事件已实时导出，窗口文本供终端上下文对照（人读）。
    crate::runtime::diagnose::trace::panic_dump();
}

/// 统一崩溃现场宏：空调用即完整转储；带参则先写一行消息再转储（可在任意点 drop-in 调试）。
#[macro_export]
macro_rules! crash_scene {
    () => {
        $crate::runtime::diagnose::scene::dump_crash()
    };
    ($($arg:tt)*) => {{
        $crate::console::_write(format_args!($($arg)*));
        $crate::put!("\n"); // 消息后换行，[scene] 标题不与消息同段紧贴
        $crate::runtime::diagnose::scene::dump_crash();
    }};
}
