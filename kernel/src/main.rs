#![no_std]
#![no_main]
#![feature(allocator_api)]

extern crate alloc;

mod console;
mod ecall;
mod lock;
mod machine;
mod memory;
mod runtime;
mod work;

use core::arch::{asm, global_asm};

use crate::machine::{Machine, Region};
use crate::memory::{allocator, manager};

global_asm!(
    ".section .text._start",
    ".globl _early_stack_top",
    ".globl _start",
    "_start:",
    "    mv   tp, a0", // hartid → tp：S-mode 读不到 M-mode CSR mhartid，入口处暂存
    "    csrc sstatus, 2", // 清 SIE：内核态恒关中断（boot 期 OpenSBI 可能遗留 SIE=1）
    "    la   sp, _early_stack_top",
    "    j    early",
);

#[unsafe(no_mangle)]
extern "C" fn early(hartid: usize, dtp: usize) -> ! {
    console::init();

    putln!("SQware Kernel booted (hart {})", hartid);

    let machine = probe(dtp);
    machine::init(machine);

    putln!(
        "dram: base @ {:#X} size = {:#X}",
        machine.dram.base,
        machine.dram.size,
    );
    putln!(
        "free: base @ {:#X} size = {:#X}",
        machine.free.base,
        machine.free.size,
    );
    putln!("hart: {}", machine.hart);

    let stack_top = machine.dram.base + machine.dram.size;

    unsafe {
        asm!(
            "mv sp, {stack_top}",
            "call {main}",
            stack_top = in(reg) stack_top,
            main = sym main,
            options(noreturn),
        );
    }
}

/// 在内核栈（DRAM 顶部）上继续启动：初始化分配器后进入 idle。
fn main() -> ! {
    allocator::init().unwrap_or_else(|e| panic!("allocator init failed: {e}"));
    manager::init().unwrap_or_else(|e| panic!("manager init failed: {e}"));
    runtime::init();

    // 阶段 C：spawn 多任务并进入首个任务（永不返回）。S-timer 由 init 武装、
    // trap_handler 内循环重武装——用户态下照常触发，驱动抢占式任务切换。
    work::init();
}

/// 内核栈大小（DRAM 顶部保留，向下增长）。
///
/// early 栈只作 bootstrap：`probe` 完成后 `main` 把 `sp` 切到 DRAM 顶部
/// （`dram_end`），此后 early 栈区域废弃，可被 bump 回收。
const KERNEL_STACK_SIZE: usize = 0x10_0000; // 1 MiB

// `_free_base`：内核镜像 + trap 栈之后的第一个可回收地址。
// early 栈（4 KiB）位于其后，但栈切换完成后即废弃，可被 bump 回收。
unsafe extern "C" {
    static _free_base: u8;
}

/// 解析设备树，返回机器设备信息 `Machine`（内存空闲区 + hart 数 + 设备 MMIO 占位）。
fn probe(dtp: usize) -> Machine {
    let fdt = unsafe { fdt::Fdt::from_ptr(dtp as *const u8) }.expect("invalid device tree blob");

    let hart = fdt.cpus().count();

    let mem = fdt
        .memory()
        .regions()
        .next()
        .expect("device tree has no /memory node");
    let dram_base = mem.starting_address.addr();
    let dram_size = mem.size.unwrap_or(0);

    let free_base = (&raw const _free_base).addr();
    // 内核栈保留在 DRAM 顶部 [dram_end - KERNEL_STACK_SIZE, dram_end)，向下增长；
    // 从 free 区剔除，避免 bump/frame 分配覆盖内核栈。
    let free_end = dram_base + dram_size - KERNEL_STACK_SIZE;
    let free_size = free_end - free_base;

    Machine {
        dram: Region::new(dram_base, dram_size),
        free: Region::new(free_base, free_size),
        hart,
        uart: Region::new(0, 0),
        plic: Region::new(0, 0),
        clint: Region::new(0, 0),
    }
}
