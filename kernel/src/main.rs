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

use log::info;

use crate::machine::{KERNEL_STACK_CANARY, kernel_stack_base};
use crate::memory::allocator;
use crate::work::unit;

global_asm!(
    ".section .text._start",
    ".globl _start",
    "_start:",
    "    mv   tp, a0", // hartid → tp：S-mode 读不到 M-mode CSR mhartid，入口处暂存
    "    csrc sstatus, 2", // 清 SIE：内核态恒关中断（boot 期 OpenSBI 可能遗留 SIE=1）
    // 主栈布局：sp = _kernel_edge + KERNEL_STACK_SIZE，栈向低地址生长。
    // 栈区不由链接脚本预留，偏移 `_stack` 是 Rust 常量（单一来源）。
    "    la   sp, _kernel_edge",
    "    la   t0, _stack",
    "    ld   t0, 0(t0)",
    "    add  sp, sp, t0",
    "    j    init",
);

#[unsafe(no_mangle)]
extern "C" fn init(hartid: usize, dtp: usize) -> ! {
    // _start 已把 sp 设到主栈顶（镜像内，无换栈）。先写主栈底 canary
    // （boot::init 进首任务前校验，检测 boot 期下溢）。
    unsafe {
        (kernel_stack_base() as *mut usize).write(KERNEL_STACK_CANARY);
    }

    console::init();

    info!("SQware Kernel booted (hart {})", hartid);

    machine::init(dtp);
    let machine = machine::info();

    info!(
        "dram: base @ {:#X} size = {:#X}",
        machine.dram.base, machine.dram.size,
    );
    info!(
        "free: base @ {:#X} size = {:#X}",
        machine.free.base, machine.free.size,
    );
    info!("hart: {} H", machine.hart);
    info!("freq: {} Hz", machine.hertz);

    // 同一主栈继续启动（永不返回）。
    main();
}

/// 在内核栈（DRAM 顶部）上继续启动：初始化分配器后进入 idle。
fn main() -> ! {
    allocator::init().unwrap_or_else(|e| panic!("allocator init failed: {e}"));
    unit::init().unwrap_or_else(|e| panic!("unit init failed: {e}"));
    runtime::clock::init(machine::info().hertz)
        .unwrap_or_else(|e| panic!("clock init failed: {e:?}"));
    // trace：静态池切分 + 核数上限防御（须在 clock 就绪后、任何 note 前）。
    runtime::trace::init();
    runtime::init();

    // boot 模块 spawn 多任务并进入首个任务（永不返回）。S-timer 由
    // runtime::init 武装、trap_handler 内循环重武装——用户态下照常触发，驱动
    // 抢占式任务切换。
    boot::init();
}
