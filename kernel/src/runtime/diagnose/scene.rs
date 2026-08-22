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
//! 输出：崩溃现场用 table::Table 渲染（label/value 对齐，列宽自动 = max cell），
//! 无堆无锁；与 lock/depend 同格式（标题行 + 表格）。地址符号化经 elftable
//! （含用户 team），详见 write_addr / addr_note。

use core::arch::asm;
use core::fmt::Write;
use riscv::interrupt::{Exception, Interrupt, Trap};
use riscv::register::{satp, scause, sepc, sscratch, sstatus, stval, stvec};

use crate::console::Sink;
use crate::memory::manager::addr::VirtAddr;
use crate::work::room::scheduler::{running_task_info, running_team_try};
use crate::work::unit::elftable;
use table::{Fmt, Table};

/// 回溯深度上限。
const BT_DEPTH: usize = 32;
/// 栈扫描窗口（从当前 sp 向上）字节数。
const BT_SCAN: usize = 4096;
/// 候选返回地址需 4 字节对齐（RISC-V 指令 2/4 字节）。
const ADDR_ALIGN: usize = 4;

/// 收行：给 Fmt 拼好的缓冲补换行，一次 flush 到控制台。
fn emit<const CAP: usize>(mut f: Fmt<CAP>) {
    let _ = writeln!(f);
    let mut sink = Sink;
    let _ = f.flush(&mut sink);
}

/// 整表渲染到控制台（无堆无锁）：Table 逐行直写缩进包装的 Sink——`[scene]`
/// 标题行顶格、表格整体缩进 2 空格（末行不补尾换行，这里补）。
fn write_table<const R: usize, const C: usize, const CAP: usize>(t: Table<R, C, CAP>) {
    let mut ind = crate::console::Indented::new(Sink);
    let _ = t.render(&mut ind);
    let mut sink = Sink;
    let _ = sink.write_str("\n");
}

/// 写地址（保留 user team 符号化）：命中出「函数+偏移」，否则裸 hex。
///
/// 注意：这点刻意不用 table::Fmt::addr——scene 需用户空间 team 符号化，而
/// Fmt::addr 走全局内核符号器（team=None）。这里写入传入的 sink，由调用方收行。
fn write_addr<W: Write>(w: &mut W, a: usize) {
    let va = VirtAddr::from_raw(a);
    let team = running_team_try();
    if let Some((name, off)) = elftable::resolve(va, team.as_deref()) {
        let _ = write!(w, "{name}+{off:#x}");
    } else {
        let _ = write!(w, "{a:#x}");
    }
}

/// 行尾符号化注解：地址的定宽值已写，命中出符号则追加「name+off」（未命中无注解）。
/// 无前缀——列间分隔由 Table 的 cell padding 统一负责（避免与语义注解叠成多层空格）。
fn addr_note<W: Write>(w: &mut W, a: usize) {
    let va = VirtAddr::from_raw(a);
    let team = running_team_try();
    if let Some((name, off)) = elftable::resolve(va, team.as_deref()) {
        let _ = write!(w, "{name}+{off:#x}");
    }
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
        // 可执行地址判定：内核域直接判，用户域用 team 表真实命中（resolve）确认
        let code = elftable::is_kernel_addr(w)
            || elftable::resolve(VirtAddr::from_raw(w), running_team_try().as_deref()).is_some();
        if w & (ADDR_ALIGN - 1) == 0 && w != prev && code {
            out[n] = w;
            n += 1;
            prev = w;
        }
        a += 8;
    }
    n
}

/// stval 解码：按 scause 的语义注解（fault 地址 / 指令位 / 断点地址）；
/// 无有价值语义时输出 Unknown（中断 / ecall / 保留码 stval 均无定义）。
fn stval_note(int: bool, code: usize) -> &'static str {
    if int {
        return "Unknown";
    }
    match code {
        0 | 1 | 4 | 5 | 6 | 7 | 12 | 13 | 15 => "faulting addr",
        2 => "illegal instruction bits",
        3 => "breakpoint addr",
        _ => "Unknown",
    }
}

/// 关键 CSR 转储（Table 渲染：三列 label/hex/注解，列宽自动 = max cell 对齐）。
/// sepc/stval/scause = 崩点；stvec/sscratch = 陷阱入口/暂存；sstatus/satp = 特权/地址空间域。
/// 注解列 = 符号化（sepc/stval/stvec）+ 解码（scause 用 riscv 枚举、stval 语义、
/// sscratch 约定、sstatus 位域、satp 用 Mode 枚举）。首行 task = 运行中任务（若有；try_lock 拿不到则跳过，表格少一行）。
fn dump_csrs() {
    let sc = scause::read();
    let (int, code) = (sc.is_interrupt(), sc.code());
    let mut t = Table::<8, 3, 160>::new();
    t.set_col_width(0, 10);
    if let Some((tid, name)) = running_task_info() {
        let row = t.open_row();
        row[0].push_str("task");
        let _ = write!(&mut row[1], "#{tid} '{name}'");
    }
    {
        let row = t.open_row();
        row[0].push_str("sepc");
        let _ = write!(&mut row[1], "{:#018x}", sepc::read());
        addr_note(&mut row[2], sepc::read());
    }
    {
        let row = t.open_row();
        row[0].push_str("stval");
        let _ = write!(&mut row[1], "{:#018x}", stval::read());
        // 符号命中 → 「sym note」单空格衔接；未命中 → 仅 note（无前缀）。
        // 不写固定「  」前缀，避免与 addr_note 叠成多余空格。
        let mut note = Fmt::<96>::new();
        {
            let va = VirtAddr::from_raw(stval::read());
            let team = running_team_try();
            if let Some((name, off)) = elftable::resolve(va, team.as_deref()) {
                let _ = write!(note, "{name}+{off:#x} ");
            }
        }
        let _ = write!(note, "{}", stval_note(int, code));
        let _ = write!(&mut row[2], "{}", note.as_str());
    }
    {
        let row = t.open_row();
        row[0].push_str("scause");
        let _ = write!(&mut row[1], "{:#018x}", sc.bits());
        // 类型化枚举（同 trap 分发）：Trap<Interrupt, Exception> Debug 即
        // 变体名（UserEnvCall / LoadPageFault / SupervisorTimer…），本身自解释，
        // 不加 int/exc 前缀（中断/异常由 bit63 隐含，hex 值列可查）。非法码回退
        // Unknown——崩溃现场不 panic；后续 CSR 行照常渲染。
        let trap: Option<Trap<Interrupt, Exception>> = sc.cause().try_into().ok();
        match trap {
            Some(Trap::Interrupt(i)) => {
                let _ = write!(&mut row[2], "{:?}", i);
            }
            Some(Trap::Exception(e)) => {
                let _ = write!(&mut row[2], "{:?}", e);
            }
            None => {
                let _ = write!(&mut row[2], "Unknown");
            }
        }
    }
    {
        let row = t.open_row();
        row[0].push_str("stvec");
        let _ = write!(&mut row[1], "{:#018x}", stvec::read().address());
        addr_note(&mut row[2], stvec::read().address());
    }
    {
        // sscratch 语义（内核约定，见 trampoline）：0 = 内核态约定；非 0 =
        // 用户态陷阱入口/线程帧相关（该值 = 曾写进 sscratch 的用户 sp 或帧自址）。
        // 注解简化为特权归属：Kernel / User。崩溃多在内核态，0 即常态。
        let scr = sscratch::read();
        let row = t.open_row();
        row[0].push_str("sscratch");
        let _ = write!(&mut row[1], "{scr:#018x}");
        if scr == 0 {
            let _ = write!(&mut row[2], "Kernel");
        } else {
            let _ = write!(&mut row[2], "User");
        }
    }
    {
        let ss = sstatus::read();
        let row = t.open_row();
        row[0].push_str("sstatus");
        let _ = write!(&mut row[1], "{:#018x}", ss.bits());
        // 注解只列非默认态：前特权模式（User/Supervisor）恒打——崩溃在用户/
        // 内核态的定位关键；布尔位置位才打缩写（SIE/SPIE/SUM/MXR/SD）；
        // FS/VS/XS 非 Off 才打短码（Off=未启用省略；短码表见各分支注释）。
        let mut note = Fmt::<160>::new();
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
        let _ = write!(&mut row[2], "{}", note.as_str());
    }
    {
        let s = satp::read();
        let row = t.open_row();
        row[0].push_str("satp");
        let _ = write!(&mut row[1], "{:#018x}", s.bits());
        let _ = write!(
            &mut row[2],
            "{:?} {:#06x} {:#013x}",
            s.mode(),
            s.asid(),
            s.ppn(),
        );
    }
    write_table(t);
}

/// GPR 转储（只打非零，命名打印）。Table 渲染两列 name/hex——列宽自动
/// = max cell 对齐（非零寄存器逐行，矩阵由 Table 游标填满）。
fn dump_gprs() {
    const NAMES: [&str; 32] = [
        "x0", "ra", "sp", "gp", "tp", "t0", "t1", "t2", "s0", "s1", "a0", "a1", "a2", "a3", "a4",
        "a5", "a6", "a7", "s2", "s3", "s4", "s5", "s6", "s7", "s8", "s9", "s10", "s11", "t3", "t4",
        "t5", "t6",
    ];
    let r = gprs();
    let mut t = Table::<32, 2, 64>::new();
    t.set_col_width(0, 10);
    for (i, name) in NAMES.iter().enumerate().skip(1) {
        if r[i] == 0 {
            continue;
        }
        let row = t.open_row();
        row[0].push_str(name);
        let _ = write!(&mut row[1], "{:#018x}", r[i]);
    }
    write_table(t);
}

/// 栈回溯转储（每条符号化）。标题行带 `[scene]` 前缀，帧行 Table 渲染两列
/// （#i / 符号化地址——列宽自动 = max cell 对齐）。
fn dump_backtrace() {
    let mut bt = [0usize; BT_DEPTH];
    let n = backtrace(&mut bt);
    let mut f = Fmt::<256>::new();
    let _ = write!(f, "[scene] backtrace ({} frames):", n);
    emit(f);
    let mut t = Table::<{ BT_DEPTH }, 2, 96>::new();
    t.set_col_width(0, 10);
    for (i, a) in bt[..n].iter().enumerate() {
        let row = t.open_row();
        let _ = write!(&mut row[0], "#{i}");
        write_addr(&mut row[1], *a);
    }
    write_table(t);
}

/// 统一崩溃现场转储：hart + 任务 + CSR + GPR + 回溯 + 事件窗口。
/// running task 并入 CSR 表首行（label=task，值列写「#tid 'name'」）——不单独
/// 打印缩进行，保持「标题行 + 顶格表格」风格统一（无行首空格）。
pub fn dump_crash() {
    let mut f = Fmt::<128>::new();
    let _ = write!(f, "[scene] crash scene, hart {}", crate::machine::hart_id());
    emit(f);
    dump_csrs();
    dump_gprs();
    dump_backtrace();
    // semihosting 下 JSON 实时流已含全部事件，文本窗口冗余——只关事件窗口，保留 CSR/GPR/回溯。
    #[cfg(not(feature = "semihosting"))]
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
        $crate::runtime::diagnose::scene::dump_crash();
    }};
}
