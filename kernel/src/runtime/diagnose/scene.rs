//! scene — 崩溃现场转储（定位错误的统一诊断）。
//!
//! 职责：回答「崩在哪个地址 / 哪条调用链」。本模块只产行
//! （`Vec<Vec<Option<String>>>`）投进 [`Report`] 的段落。
//!
//! 现场语义：GPR 是处理器已压栈损坏的现场；真正可定位的是 CSR 的 sepc/scause/stval
//! （trap 进入后持续有效）与栈回溯。
//!
//! 回溯 = 无帧指针启发式：扫描当前栈区间，收集可执行地址的候选返回地址（去重、
//! 深度封顶）。每条经 `elftable::resolve` 符号化，未命中打印裸 hex。

use core::arch::asm;

use alloc::format;
use alloc::string::{String, ToString};
use alloc::sync::Arc;
use alloc::vec;
use alloc::vec::Vec;
use riscv::interrupt::{Exception, Interrupt, Trap};
use riscv::register::{satp, scause, sepc, sscratch, sstatus, stval, stvec};

use crate::layout::STACK_WINDOW_SIZE;
use crate::memory::PAGE_SIZE;
use crate::memory::manager::addr::{PhysAddr, VirtAddr};
use crate::memory::manager::entry::PteFlags;
use crate::runtime::diagnose::report::Report;
use crate::runtime::switcher::context::{Gprs, TrapContext};
use crate::work::room::conductor::core::ident;
use crate::work::unit::elftable::{self, ElfTable};

/// 内核团队符号表（`elftable::routed*` 的内核侧来源；随内核团队挂载，未装配 → None）。
/// 路由决策归消费方（本模块），elftable 模块本身不依赖 team（环已拆）。
fn ktbl() -> Option<&'static ElfTable> {
    crate::work::unit::team::kernel()?.elftable.as_deref()
}

fn utbl() -> Option<Arc<ElfTable>> {
    ident()?.elftable()
}

/// 回溯深度上限。
const BT_DEPTH: usize = 32;
/// 栈扫描窗口（从当前 sp 向上）字节数。
const BT_SCAN: usize = 4096;
/// 候选返回地址需 4 字节对齐（RISC-V 指令 2/4 字节）。
const ADDR_ALIGN: usize = 4;
/// 栈窗口 slot 步长（守护页 + 栈体，见 space::window::StackWindow::claim）——
/// 内核团队任务栈段顶对齐基准。
const STACK_SLOT: usize = crate::layout::TASK_STACK_SIZE + crate::layout::TASK_STACK_GUARD;

/// 定宽 hex 文本（{:#018x}）——值列的通用形态。
fn hex(x: usize) -> String {
    format!("{x:#018x}")
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

/// 栈回溯：**帧指针链**为主（内核 `-Cforce-frame-pointers=yes` 保证 FP 存在），
/// 脱链后 fallback 到启发式扫描（`core::panicking` 等预编译库无 FP，panic 链
/// 可能在它们处断——其外层仍是带 FP 的内核栈帧，由启发式接续）。
///
/// FP 链（标准 RV64 布局）：`[fp]` = 调用者 fp、`[fp+8]` = 返回地址 ra；栈向
/// 下生长 → fp 逐帧单调递增，非单调/越界/零即脱链。每帧取 ra（非零、不重复）
/// 入槽。启发式（兜底）：从链断点起按 8 字节步进向上扫描，收集「4 对齐 +
/// 非相邻重复 + 能解析进符号区间」的可执行候选。
fn kbacktrace(out: &mut [usize; BT_DEPTH]) -> usize {
    // 起扫点：panic 归巢（halt::home）后当前 sp/fp 是救援栈组稿链——kbt 须回
    // 原始现场（SCENE = home 落盘的崩点 sp/fp）扫原栈；crash_scene! 直调（无
    // 归巢、无 SCENE）→ 读当前 sp/fp（与历史行为一致）。
    let (sp, fp) = match crate::runtime::diagnose::halt::scene() {
        (0, 0) => {
            let (sp, fp): (usize, usize);
            unsafe {
                asm!("mv {0}, sp", out(reg) sp);
                asm!("mv {0}, s0", out(reg) fp);
            }
            (sp, fp)
        }
        s => s,
    };
    // 扫描上界钳制：崩溃现场可能运行在「段顶之上是刻意未映射 guard 页」的栈上，
    // 扫描窗越过段顶即读缺页 → 嵌套 panic → 现场静默消失（幽灵 panic）。high 钳到
    // 两类段顶（trap 栈 / 内核任务栈 slot）；其余栈上方为已映射 DRAM，退回原行为。
    // trap 栈按 sp 纯算术反推（固定 VA 窗口），不依赖 hart_id/元数据表。
    let high = {
        let trap_top = crate::runtime::switcher::trap::trap_stack_hart(sp)
            .map(crate::runtime::switcher::trap::trap_stack_edge)
            .map(|e| e.as_usize());
        // 栈窗顶锚于用户空间上界之下：[upper − STACK_WINDOW_SIZE, upper)。
        let stack_base = crate::memory::manager::mode::upper().as_usize() - STACK_WINDOW_SIZE;
        let stack_top = (sp >= stack_base && sp < stack_base + STACK_WINDOW_SIZE)
            .then(|| (sp & !(STACK_SLOT - 1)) + STACK_SLOT);
        match trap_top.or(stack_top) {
            Some(t) => sp.saturating_add(BT_SCAN).min(t),
            None => sp.saturating_add(BT_SCAN),
        }
    };
    let mut n = 0usize;
    let mut prev = 0usize;
    // ① 帧指针链：每帧取 ra 与调用者 fp（单调增，栈向下生长）。
    let mut f = fp;
    let mut last = fp; // 最后有效帧地址（fallback 扫描起点）
    while n < BT_DEPTH && f >= sp && f + 16 <= high {
        last = f;
        // SAFETY: f 落在 [sp, sp+BT_SCAN)（S 态直读本线程栈，无副作用）。
        let caller = unsafe { *(f as *const usize) };
        let ra = unsafe { *((f + 8) as *const usize) };
        if ra != 0 && ra != prev {
            out[n] = ra;
            n += 1;
            prev = ra;
        }
        if caller == 0 || caller <= f {
            break; // 脱链：根帧 / 帧损坏 / 无 FP 链段（如 core::panicking）
        }
        f = caller;
    }
    // ② 启发式兜底：从链断点继续向上扫描，接续断链外层的内核栈帧。
    let mut a = core::cmp::max(last + 8, sp & !7usize);
    while a < high && n < BT_DEPTH {
        // SAFETY: 只读本线程栈区间（S 态直读恒等映射内存，无副作用）。
        let w = unsafe { (a as *const usize).read_volatile() };
        // 可执行候选 = 「能解析进符号区间」的地址（routed 按域选表 + lookup
        // 上界约束）；栈上数据字落空——不收宽镜像区间误报。
        let code = elftable::routed(
            VirtAddr::from_raw(w),
            ktbl(),
            ident().as_ref().and_then(|i| i.elftable()).as_deref(),
        )
        .is_some();
        if w & (ADDR_ALIGN - 1) == 0 && w != prev && code {
            out[n] = w;
            n += 1;
            prev = w;
        }
        a += 8;
    }
    n
}

/// 用户侧回溯：崩溃时 running 任务的**用户 trap 帧**里有该任务最近一次用户态
/// 现场。取用户 sp 按用户页表逐页安全读栈窗口，收集可执行候选返回地址（判定 =
/// 本任务自己的符号表命中 + 对齐 + 去重）。尽力而为：帧拿不到/现场非用户态 /
/// 栈页不可读 → 0 帧（不渲染 ubt 段）。返回 (帧数, 任务符号表 Arc)。
fn ubacktrace(out: &mut [usize; BT_DEPTH]) -> (usize, Option<Arc<ElfTable>>) {
    let Some(info) = ident() else {
        return (0, None);
    };
    let tbl = info.elftable();
    // trap 仅 Live 轴可读（Current::Last 不含 trap——帧可能已回收）；取不到 → 0 帧。
    let Some(trap) = info.trap() else {
        return (0, tbl);
    };
    // SAFETY: Live = 本核在跑任务，帧未回收；帧 PA 在用户 Frame 窗口（DRAM，恒等
    // 映射）；崩溃现场读只读、其余核冻结。
    let frame = unsafe { &*(trap.pa.as_usize() as *const TrapContext) };
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
    // DRAM 恒等区守卫（上界随机器 dram 取）：walk_raw 逐级 PA 都过此校验——用户
    // satp 若被覆写为坏值，裸读会 fault → 嵌套 panic；此处拦下，跳页继续。未注入
    // 机器信息 → 退回保守上界。
    let in_dram = |pa: PhysAddr| {
        (0x8000_0000..crate::machine::dram_end().unwrap_or(0x9000_0000)).contains(&pa.as_usize())
    };
    while page < high && n < BT_DEPTH {
        let Some((pa0, flags)) = crate::memory::manager::table::TableNode::walk_raw(
            PhysAddr::from_raw(root_ppn << 12),
            VirtAddr::from_raw(page),
            in_dram,
        ) else {
            page += PAGE_SIZE;
            continue;
        };
        if !flags.contains(PteFlags::R) {
            // 无读权（诊断只读栈）——本页不可读，跳页继续。
            page += PAGE_SIZE;
            continue;
        }
        let start = core::cmp::max(page, sp & !7);
        let end = core::cmp::min(page + PAGE_SIZE, high);
        let mut a = start;
        while a < end && n < BT_DEPTH {
            // SAFETY: 该页已 walk 命中且带 R；offset 恒在页内，S 态直读。
            let w = unsafe { ((pa0.as_usize() + (a - page)) as *const usize).read_volatile() };
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

/// CSR 段行集（首行表头）：sepc/stval/scause = 崩点；stvec/sscratch = 陷阱
/// 入口/暂存；sstatus/satp = 特权/地址空间域。task 行 = 运行中任务（若有；
/// try_lock 拿不到则跳过）。注解列 = 符号化 + 解码。
fn csr_rows() -> Vec<Vec<Option<String>>> {
    let mut rows: Vec<Vec<Option<String>>> = vec![
        vec![None, Some("hex".into()), Some("note".into())], // 首行表头
    ];
    let sc = scause::read();
    let (int, code) = (sc.is_interrupt(), sc.code());
    if let Some(i) = ident() {
        rows.push(vec![
            Some(format!("#{}", i.id())),
            Some(format!("'{}'", i.name())),
            None,
        ]);
    }
    rows.push(vec![
        Some("sepc".into()),
        Some(hex(sepc::read())),
        Some(elftable::routed_symbol(
            VirtAddr::from_raw(sepc::read()),
            ktbl(),
            utbl().as_deref(),
        )),
    ]);
    {
        // 符号命中 → 「sym note」单空格衔接；未命中 → 仅 stval 语义。
        let a = stval::read();
        let n = if let Some((name, off)) = elftable::routed(
            VirtAddr::from_raw(a),
            ktbl(),
            ident().as_ref().and_then(|i| i.elftable()).as_deref(),
        ) {
            format!("{name}+{off:#x} {}", stval_note(int, code))
        } else {
            stval_note(int, code).to_string()
        };
        rows.push(vec![Some("stval".into()), Some(hex(a)), Some(n)]);
    }
    {
        // 类型化枚举：变体名自解释；非法码回退 Unknown。
        let trap: Option<Trap<Interrupt, Exception>> = sc.cause().try_into().ok();
        let note = match trap {
            Some(Trap::Interrupt(i)) => format!("{i:?}"),
            Some(Trap::Exception(e)) => format!("{e:?}"),
            None => "Unknown".to_string(),
        };
        rows.push(vec![
            Some("scause".into()),
            Some(hex(sc.bits())),
            Some(note),
        ]);
    }
    rows.push(vec![
        Some("stvec".into()),
        Some(hex(stvec::read().address())),
        Some(elftable::routed_symbol(
            VirtAddr::from_raw(stvec::read().address()),
            ktbl(),
            None,
        )),
    ]);
    {
        // sscratch 约定：内核态 = 本 hart trap 帧 VA（HART_FRAME_BASE +
        // hart·PAGE，可反推 hart）；用户态 = 当前线程帧 self_va（team 帧区）。
        // 值域判定：hart 帧区 → 内核态帧（可推 hart）；team 帧区 → 用户帧。
        let scr = sscratch::read();
        let kfb = crate::layout::HART_FRAME_BASE.as_usize();
        let n = if scr == 0 {
            "Kernel (sscratch=0)".to_string()
        } else if scr >= kfb && scr < kfb + crate::machine::MAX_HART_SLOTS * PAGE_SIZE {
            format!("Kernel frame (hart {})", (scr - kfb) / PAGE_SIZE)
        } else if scr >= crate::layout::TEAM_FRAME_BASE.as_usize()
            && scr < crate::layout::HART_FRAME_BASE.as_usize()
        {
            "User frame".into()
        } else {
            "other".into()
        };
        rows.push(vec![Some("sscratch".into()), Some(hex(scr)), Some(n)]);
    }
    {
        // 注解只列非默认态：前特权模式恒打；布尔位置位才打缩写（SIE/SPIE/
        // SUM/MXR/SD）；FS/VS/XS 非 Off 才打短码。
        let ss = sstatus::read();
        let mut note = format!("{:?}", ss.spp());
        if ss.sie() {
            note.push_str(" SIE");
        }
        if ss.spie() {
            note.push_str(" SPIE");
        }
        if ss.fs() != riscv::register::mstatus::FS::Off {
            note.push_str(match ss.fs() {
                riscv::register::mstatus::FS::Initial => " FS FI",
                riscv::register::mstatus::FS::Clean => " FS FC",
                riscv::register::mstatus::FS::Dirty => " FS FD",
                riscv::register::mstatus::FS::Off => " FS FO",
            });
        }
        if ss.vs() != riscv::register::mstatus::VS::Off {
            note.push_str(match ss.vs() {
                riscv::register::mstatus::VS::Initial => " VS VI",
                riscv::register::mstatus::VS::Clean => " VS VC",
                riscv::register::mstatus::VS::Dirty => " VS VD",
                riscv::register::mstatus::VS::Off => " VS VO",
            });
        }
        if ss.xs() != riscv::register::mstatus::XS::AllOff {
            note.push_str(match ss.xs() {
                riscv::register::mstatus::XS::NoneDirtyOrClean => " XS XI",
                riscv::register::mstatus::XS::NoneDirtySomeClean => " XS XC",
                riscv::register::mstatus::XS::SomeDirty => " XS XD",
                riscv::register::mstatus::XS::AllOff => " XS XA",
            });
        }
        if ss.sum() {
            note.push_str(" SUM");
        }
        if ss.mxr() {
            note.push_str(" MXR");
        }
        if ss.sd() {
            note.push_str(" SD");
        }
        rows.push(vec![
            Some("sstatus".into()),
            Some(hex(ss.bits())),
            Some(note),
        ]);
    }
    {
        let s = satp::read();
        let note = format!("{:?} {:#06x} {:#013x}", s.mode(), s.asid(), s.ppn());
        rows.push(vec![Some("satp".into()), Some(hex(s.bits())), Some(note)]);
    }
    rows
}

/// GPR 段行集（首行表头，其后只打非零）：label/hex 两槽。
fn gpr_rows() -> Vec<Vec<Option<String>>> {
    const NAMES: [&str; 32] = [
        "x0", "ra", "sp", "gp", "tp", "t0", "t1", "t2", "s0", "s1", "a0", "a1", "a2", "a3", "a4",
        "a5", "a6", "a7", "s2", "s3", "s4", "s5", "s6", "s7", "s8", "s9", "s10", "s11", "t3", "t4",
        "t5", "t6",
    ];
    let mut rows: Vec<Vec<Option<String>>> = vec![
        vec![None, Some("hex".into())], // 首行表头
    ];
    let r = gprs();
    for (i, name) in NAMES.iter().enumerate().skip(1) {
        if r[i] == 0 {
            continue;
        }
        rows.push(vec![Some((*name).into()), Some(hex(r[i]))]);
    }
    rows
}

/// 统一崩溃现场组稿：CSR 三列表 + GPR 两列表 + 回溯表（内核栈 kbt + 用户栈 ubt）。
/// 末尾倒出每 hart 最近事件窗口。
pub fn dump_crash(r: &mut Report) {
    // 探针：panic 现场 drop-in 完整性体检——越界写破坏用户符号表/相邻活块
    // 时自报。两者均纯读零分配、只经 putln! 直写控制台——panic 现场安全，
    // 且不截断本次转储。
    #[cfg(debug_assertions)]
    {
        if let Some(et) = ident().as_ref().and_then(|i| i.elftable()).as_ref() {
            et.check_integrity();
        }
        let _ = crate::memory::allocator::fence::ledger::LEDGER.sweep_canaries();
    }
    // 投稿：CSR/GPR/回溯段入报告（[scene] 标题挂首段，其余段空标题同段落）。
    r.paragraph(
        "csr",
        Some(format!(
            "[scene] crash scene, hart {}",
            crate::machine::hart_id()
        )),
    )
    .items
    .extend(csr_rows());
    r.paragraph("gpr", None).items.extend(gpr_rows());

    let mut bt = [0usize; BT_DEPTH];
    let n = kbacktrace(&mut bt);
    {
        // 行 = #i / 定宽 hex / 符号。
        let mut rows: Vec<Vec<Option<String>>> = vec![vec![
            Some("#".into()),
            Some("hex".into()),
            Some("sym".into()),
        ]];
        for (i, a) in bt[..n].iter().enumerate() {
            rows.push(vec![
                Some(format!("#{i}")),
                Some(hex(*a)),
                Some(elftable::routed_symbol(
                    VirtAddr::from_raw(*a),
                    ktbl(),
                    None,
                )),
            ]);
        }
        r.paragraph("kbt", None).items.extend(rows);
    }

    // 用户侧回溯（kbt = 内核栈；ubt = 用户栈）——被中断上下文为用户态、能取到
    // running 任务帧时附加；0 帧不渲染。符号化用任务自己的表。
    let mut ubt = [0usize; BT_DEPTH];
    let (m, tbl_arc) = ubacktrace(&mut ubt);
    if m > 0 {
        // 行 = #i / 定宽 hex / 符号（符号化用任务自己的表）。
        let mut rows: Vec<Vec<Option<String>>> = vec![vec![
            Some("#".into()),
            Some("hex".into()),
            Some("sym".into()),
        ]];
        for (i, a) in ubt[..m].iter().enumerate() {
            rows.push(vec![
                Some(format!("#{i}")),
                Some(hex(*a)),
                Some(elftable::symbol(VirtAddr::from_raw(*a), tbl_arc.as_deref())),
            ]);
        }
        r.paragraph("ubt", None).items.extend(rows);
    }

    // 每 hart 最近事件窗口（人读对照）。
    crate::runtime::diagnose::trace::panic_dump(r);
}

/// 统一崩溃现场宏：空调用即完整转储（自建报告、成册、印发——可在任意点
/// drop-in 调试）；带参则先写一行消息再转储。
#[macro_export]
macro_rules! crash_scene {
    () => {{
        let mut __r = $crate::runtime::diagnose::report::Report::default();
        $crate::runtime::diagnose::scene::dump_crash(&mut __r);
        let __sealed = __r.seal();
        let mut __sink = $crate::console::Sink;
        $crate::runtime::diagnose::render::render(__sealed, &mut __sink, 2);
        #[cfg(feature = "semihosting")]
        $crate::runtime::diagnose::export::export(__sealed);
    }};
    ($($arg:tt)*) => {{
        $crate::console::_write(format_args!($($arg)*));
        $crate::put!("\n"); // 消息后换行，[scene] 标题不与消息同段紧贴
        $crate::crash_scene!();
    }};
}
