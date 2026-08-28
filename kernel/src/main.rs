#![no_std]
#![no_main]
#![feature(allocator_api)]

extern crate alloc;

mod boot;
mod console;
mod health;
mod layout;
mod lock;
mod machine;
mod memory;
mod runtime;
mod work;

use core::arch::global_asm;

use crate::memory::allocator;
use crate::runtime::chrono::clock;
use crate::runtime::diagnose::trace;
use crate::runtime::switcher::trap;
use crate::work::unit;

global_asm!(
    ".section .text._start",
    ".globl _start",
    "_start:",
    "    la   t0, PER_HART", // &PER_HART[0]（恒等映射，Bare 下 PC 相对即物理地址）
    "    slli t1, a0, 4",    // hartid · 16（PerHart 槽宽，编译期断言锁死）
    "    add  tp, t0, t1",   // tp = 本 hart PerHart 指针（入口约定，见 `hart_id()`）
    "    csrc sstatus, 2",
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
    clock::init().unwrap_or_else(|e| panic!("clock init failed: {e}"));
    trace::init().unwrap_or_else(|e| panic!("trace init failed: {e}"));
    trap::init();
    boot::banner();
    boot::init();
}
