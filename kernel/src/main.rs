#![no_std]
#![no_main]
#![feature(allocator_api)]

extern crate alloc;

mod console;
mod context;
mod ecall;
mod lock;
mod memory;
mod panicking;
mod runtime;

use core::arch::{asm, global_asm};

use crate::memory::allocator::{self, Region};

global_asm!(
    ".section .text._start",
    ".globl _early_stack_top",
    ".globl _start",
    "_start:",
    "    la   sp, _early_stack_top",
    "    j    main",
);

#[unsafe(no_mangle)]
extern "C" fn main(hartid: usize, dtb_addr: usize) {
    console::init();

    putln!("SQware kernel booted (hart {})", hartid);
    putln!("put/putln output via legacy sbi_console_putchar");
    log::info!("log crate routed to console");

    // 解析设备树，注入物理内存池区域并初始化分配器。
    let (region, hart_count) = probe_memory(dtb_addr);
    unsafe { allocator::init(region, hart_count) };
    putln!(
        "memory: {:#x}..{:#x} ({} harts)",
        region.base,
        region.end,
        hart_count
    );

    loop {
        unsafe {
            asm!("wfi");
        }
    }
}

// 链接脚本 `link.ld` 中内核镜像 + trap/early 栈之后的最高已用地址。
// bump 分配器从它（按页对齐）向上分配，与向下增长的 early 栈互不重叠。
unsafe extern "C" {
    static _early_stack_top: u8;
}

/// 解析设备树，返回可用的物理内存空闲区间 `[base, end)` 与 hart 数。
fn probe_memory(dtb_addr: usize) -> (Region, usize) {
    let fdt =
        unsafe { fdt::Fdt::from_ptr(dtb_addr as *const u8) }.expect("invalid device tree blob");

    let hart_count = fdt.cpus().count();

    let mem = fdt
        .memory()
        .regions()
        .next()
        .expect("device tree has no /memory node");
    let dram_base = mem.starting_address as usize;
    let dram_size = mem.size.unwrap_or(0);

    // 空闲区起点取内核镜像 + 栈之后的 _early_stack_top（页对齐），
    // 终点取设备树给出的 DRAM 末尾。不能从 dram_base 整段开始，
    // 否则会与 0x80200000 的内核镜像及栈重叠。
    let free_base = unsafe { &_early_stack_top as *const u8 as usize };
    let free_end = dram_base + dram_size;

    (
        Region {
            base: free_base,
            end: free_end,
        },
        hart_count,
    )
}
