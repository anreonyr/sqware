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

use crate::machine::{Machine, Region};
use crate::memory::{allocator, manager};

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

    let machine = probe(dtp);
    machine::init(machine);

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
    manager::init().unwrap_or_else(|e| panic!("manager init failed: {e}"));
    runtime::clock::init(machine::get().hertz)
        .unwrap_or_else(|e| panic!("clock init failed: {e:?}"));
    // trace：静态池切分 + 核数上限防御（须在 clock 就绪后、任何 note 前）。
    runtime::trace::init();
    runtime::init();

    // boot 模块 spawn 多任务并进入首个任务（永不返回）。S-timer 由
    // runtime::init 武装、trap_handler 内循环重武装——用户态下照常触发，驱动
    // 抢占式任务切换。
    boot::init();
}

/// 主内核栈布局（镜像内单一引导栈；栈大小由 Rust 常量单一来源）：
///   `_kernel_edge`             镜像结束（页对齐，链接脚本唯一锚点）
///   [主栈区 KERNEL_STACK_SIZE] 向下生长，栈底 = `_kernel_edge`，栈顶 = +size
/// free 区起点 = 栈顶。无独立 guard 帧——主栈是 boot 短命栈，下溢由栈底
/// canary 兜底（boot::init 进首任务前校验）。
pub(crate) const KERNEL_STACK_SIZE: usize = 0x10_0000; // 1 MiB

/// 主内核栈 canary 值（写在栈底，`boot::init` 进首任务前校验）。
pub(crate) const KERNEL_STACK_CANARY: usize = 0x600D_CAFE_51A7_0D1E;

/// 镜像结束地址（链接脚本 `_kernel_edge`）——栈与 free 区布局的唯一基准。
pub(crate) fn kernel_edge() -> usize {
    (&raw const _kernel_edge).addr()
}
unsafe extern "C" {
    /// 内核镜像结束锚点（见 link.ld）。栈区与 free 区都从它推导。
    static _kernel_edge: u8;
}

/// 主栈区偏移（栈大小）——`_start` 汇编加载它算出栈顶 sp。
/// no_mangle 暴露为符号，global_asm `la t0,_stack; ld t0,0(t0)` 读取，
/// 栈大小单一来源（此处由 Rust 常量推导），链接脚本不写栈布局。
#[unsafe(no_mangle)]
static _stack: usize = KERNEL_STACK_SIZE;

/// 主内核栈底地址（canary 所在）。
pub(crate) fn kernel_stack_base() -> usize {
    kernel_edge()
}

/// 主内核栈顶地址（= free 区起点）。
pub(crate) fn kernel_stack_edge() -> usize {
    kernel_edge() + KERNEL_STACK_SIZE
}

/// 读取 DTB `/cpus` 的 timebase-frequency（Hz）；缺失/非法长度返回 0
/// （clock::init 会以 ClockError::NoTimebase 拒绝 0）。
fn timebase_of(fdt: &fdt::Fdt) -> usize {
    fdt.find_node("/cpus")
        .and_then(|n| n.property("timebase-frequency"))
        .map(|p| match p.value.len() {
            4 => u32::from_be_bytes([p.value[0], p.value[1], p.value[2], p.value[3]]) as usize,
            8 => {
                let b = p.value;
                u64::from_be_bytes([b[0], b[1], b[2], b[3], b[4], b[5], b[6], b[7]]) as usize
            }
            _ => 0,
        })
        .unwrap_or(0)
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
    let timebase = timebase_of(&fdt);

    let free_base = crate::kernel_stack_edge();
    // 主内核栈位于镜像内（guard + 主栈区），整个空闲区
    // （free_base = 栈顶 .. dram_end）均可分配。
    let free_end = dram_base + dram_size;
    let free_size = free_end - free_base;

    Machine {
        dram: Region::new(dram_base, dram_size),
        free: Region::new(free_base, free_size),
        hart,
        hertz: timebase,
        uart: Region::new(0, 0),
        plic: Region::new(0, 0),
        clint: Region::new(0, 0),
    }
}
