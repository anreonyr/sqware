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

use core::arch::asm;
use riscv::register::{satp, scause, sepc, sstatus, stval};

use crate::console::_write;
use crate::memory::manager::addr::VirtAddr;
use crate::work::unit::elftable;
use crate::work::scheduler::{running_task_info, running_team_try};

/// 回溯深度上限。
const BT_DEPTH: usize = 32;
/// 栈扫描窗口（从当前 sp 向上）字节数。
const BT_SCAN: usize = 4096;
/// 候选返回地址需 4 字节对齐（RISC-V 指令 2/4 字节）。
const ADDR_ALIGN: usize = 4;

/// 打印地址：经 elftable::resolve 符号化（命中出「函数+偏移」），否则裸 hex。
fn print_addr(a: usize) {
    let va = VirtAddr::from_raw(a);
    let team = running_team_try();
    if let Some((name, off)) = elftable::resolve(va, team.as_deref()) {
        _write(format_args!("{name}+{off:#x}"));
    } else {
        _write(format_args!("{a:#x}"));
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

/// 关键 CSR 转储（sepc/stval/scause = 崩点；sstatus/satp = 特权/地址空间域）。
fn dump_csrs() {
    let sc = scause::read();
    let kind = if sc.is_interrupt() { "int" } else { "exc" };
    _write(format_args!("[scene] sepc="));
    print_addr(sepc::read());
    _write(format_args!("  stval="));
    print_addr(stval::read());
    _write(format_args!(
        "  scause={} ({})  sstatus={:#x}  satp={:#x}\n",
        sc.code(),
        kind,
        sstatus::read().bits(),
        satp::read().bits(),
    ));
}

/// GPR 转储（只打非零，命名打印）。
fn dump_gprs() {
    const NAMES: [&str; 32] = [
        "x0", "ra", "sp", "gp", "tp", "t0", "t1", "t2", "s0", "s1", "a0", "a1", "a2", "a3", "a4",
        "a5", "a6", "a7", "s2", "s3", "s4", "s5", "s6", "s7", "s8", "s9", "s10", "s11", "t3",
        "t4", "t5", "t6",
    ];
    let r = gprs();
    _write(format_args!("[scene] gpr:"));
    for (i, name) in NAMES.iter().enumerate().skip(1) {
        if r[i] != 0 {
            _write(format_args!(" {}={:#x}", name, r[i]));
        }
    }
    _write(format_args!("\n"));
}

/// 栈回溯转储（每条符号化）。
fn dump_backtrace() {
    let mut bt = [0usize; BT_DEPTH];
    let n = backtrace(&mut bt);
    _write(format_args!("[scene] backtrace ({} frames):\n", n));
    for (i, a) in bt[..n].iter().enumerate() {
        _write(format_args!("[scene]   #{i}: "));
        print_addr(*a);
        _write(format_args!("\n"));
    }
}

/// 统一崩溃现场转储：hart + 任务 + CSR + GPR + 回溯 + 事件窗口。
pub fn dump_crash() {
    _write(format_args!(
        "[scene] crash scene, hart {}\n",
        crate::machine::hart_id()
    ));
    if let Some((tid, name)) = running_task_info() {
        _write(format_args!("[scene] running task #{tid} '{name}'\n"));
    }
    dump_csrs();
    dump_gprs();
    dump_backtrace();
    crate::runtime::trace::panic_dump();
}

/// 统一崩溃现场宏：空调用即完整转储；带参则先写一行消息再转储（可在任意点 drop-in 调试）。
#[macro_export]
macro_rules! crash_scene {
    () => {
        $crate::runtime::scene::dump_crash()
    };
    ($($arg:tt)*) => {{
        $crate::console::_write(format_args!($($arg)*));
        $crate::runtime::scene::dump_crash();
    }};
}
