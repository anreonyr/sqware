#![no_std]
#![no_main]
#![feature(allocator_api)]

extern crate alloc;

mod boot;
mod console;
mod health;
mod lock;
mod machine;
mod memory;
mod runtime;
mod work;

use core::arch::global_asm;
use core::sync::atomic::{AtomicUsize, Ordering};

use crate::machine::{KERNEL_STACK_CANARY, kernel_stack_base, kernel_stack_edge};
use crate::memory::allocator;
use crate::runtime::chrono::clock;
use crate::runtime::diagnose::trace;
use crate::runtime::switcher::trap;
use crate::work::unit;

/// boot 主栈峰值（字节，boot 路径各里程碑采样；供「缩小主栈」定标）。
static MAIN_STACK_PEAK: AtomicUsize = AtomicUsize::new(0);

/// 主栈用量采样点：读当前 sp，折算已用字节并记峰值。非内联保证读数真实。
///
/// 采样点 = boot 路径各里程碑（main 的每段 init 之后、boot::init 的
/// spawn/boot_harts 之后）；峰值的可信度取决于采样点是否覆盖最深调用链
/// （format!/装载/health 等在两点之间深入）。boot 末尾打印峰值核对。
#[inline(never)]
pub(crate) fn probe_main_stack() {
    let sp: usize;
    // SAFETY: 纯读本 hart sp 寄存器，无副作用。
    unsafe { core::arch::asm!("mv {}, sp", out(reg) sp) };
    let used = kernel_stack_edge().saturating_sub(sp);
    MAIN_STACK_PEAK.fetch_max(used, Ordering::Relaxed);
}

/// boot 结束后主栈峰值字节数（探测结果；供定标 KERNEL_STACK_SIZE）。
pub(crate) fn main_stack_peak() -> usize {
    MAIN_STACK_PEAK.load(Ordering::Relaxed)
}

global_asm!(
    ".section .text._start",
    ".globl _start",
    "_start:",
    "    mv   tp, a0", // hartid → tp：S-mode 读不到 M-mode CSR mhartid，入口处暂存
    "    csrc sstatus, 2", // 清 SIE：内核态恒关中断（boot 期 OpenSBI 可能遗留 SIE=1）
    // 主栈布局：sp = _kernel_edge + KERNEL_STACK_SIZE，栈向低地址生长。
    "    la   sp, _kernel_edge",
    "    ld   t0, _canary",
    "    sd   t0, 0(sp)",
    "    la   t0, _stack",
    "    ld   t0, 0(t0)",
    "    add  sp, sp, t0",
    "    j    main",
);

#[unsafe(no_mangle)]
extern "C" fn main(_hartid: usize, dtp: usize) -> ! {
    probe_main_stack(); // 基线（_start 后、main 序言深处）
    console::init();
    probe_main_stack();
    machine::init(dtp);
    probe_main_stack();
    allocator::init().unwrap_or_else(|e| panic!("allocator init failed: {e}"));
    probe_main_stack();
    unit::init().unwrap_or_else(|e| panic!("unit init failed: {e}"));
    probe_main_stack();
    clock::init().unwrap_or_else(|e| panic!("clock init failed: {e}"));
    // trace：动态 ring（size_of 精确预算，在 clock 就绪后、任何 note 前初始化）。
    trace::init().unwrap_or_else(|e| panic!("trace init failed: {e}"));
    probe_main_stack();
    trap::init();
    // 启动横幅：机器块 + 陷阱布局块连打。trap 栈的 top/bottom 须在 trap::init
    // 之后取，故 banner 放这里。
    boot::banner();
    probe_main_stack();

    // 启动多任务并进入首个任务。
    boot::init();
}
