#![no_std]
#![no_main]
#![feature(allocator_api)]

extern crate alloc;

mod boot;
mod console;
mod lock;
mod machine;
mod memory;
mod runtime;
mod work;

use core::arch::global_asm;

use crate::machine::{KERNEL_STACK_CANARY, kernel_stack_base};
use crate::memory::allocator;
use crate::runtime::{clock, trace, trap};
use crate::work::unit;

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
    console::init();

    machine::init(dtp);

    allocator::init().unwrap_or_else(|e| panic!("allocator init failed: {e}"));
    unit::init().unwrap_or_else(|e| panic!("unit init failed: {e}"));
    clock::init().unwrap_or_else(|e| panic!("clock init failed: {e:?}"));
    // trace：静态池切分 + 核数上限防御（须在 clock 就绪后、任何 note 前）。
    trace::init();
    trap::init();
    // 启动横幅：机器块 + 陷阱布局块连打。trap 栈为堆分配、其 top/bottom 须在
    // trap::init（分配并填充 TRAP_STACKS）之后取，故 banner 放这里。
    boot::banner();

    // boot 模块 spawn 多任务并进入首个任务（永不返回）。S-timer 由
    // runtime::init 武装、trap_handler 内循环重武装——用户态下照常触发，驱动
    // 抢占式任务切换。
    boot::init();
}
