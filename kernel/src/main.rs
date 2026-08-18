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

use core::arch::{asm, global_asm};

use log::info;

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

    let stack_top = machine.dram.base + machine.dram.size;

    // 主内核栈底写 canary（boot 期自下而上生长；boot::init 进首任务前校验）。
    let stack_bottom = stack_top - KERNEL_STACK_SIZE;
    unsafe {
        (stack_bottom as *mut usize).write(KERNEL_STACK_CANARY);
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
    runtime::clock::init(machine::get().hertz)
        .unwrap_or_else(|e| panic!("clock init failed: {e:?}"));
    runtime::init();

    // boot 模块 spawn 多任务并进入首个任务（永不返回）。S-timer 由
    // runtime::init 武装、trap_handler 内循环重武装——用户态下照常触发，驱动
    // 抢占式任务切换。
    boot::init();
}

/// 内核栈大小（DRAM 顶部保留，向下增长）。
///
/// 栈底之下还保留一页 guard（未映射，见 `manager::init` 的开栈 guard unmap）：
/// 向下溢出越过 guard 页时触发缺页，而非静默踩进 free 区。栈底本身写有
/// canary（见 [`KERNEL_STACK_CANARY`]），`boot::init` 离开本栈前校验——boot 期
/// 溢出即使未越过 guard 也会在此暴露。
///
/// early 栈只作 bootstrap：`probe` 完成后 `main` 把 `sp` 切到 DRAM 顶部
/// （`dram_end`），此后 early 栈区域废弃，可被 bump 回收。
pub(crate) const KERNEL_STACK_SIZE: usize = 0x10_0000; // 1 MiB

/// 主内核栈 canary 值（写在栈底，`boot::init` 进首任务前校验）。
/// boot 期主内核栈自上而下生长，栈底紧邻 guard 页之上。
pub(crate) const KERNEL_STACK_CANARY: usize = 0x600D_CAFE_51A7_0D1E;

/// 主内核栈底地址（canary 所在，guard 页之上；DRAM 顶部栈区下缘）。
pub(crate) fn kernel_stack_bottom() -> usize {
    let m = crate::machine::get();
    m.dram.base + m.dram.size - KERNEL_STACK_SIZE
}

// `_free_base`：内核镜像之后的第一个可回收地址（per-hart trap 栈为动态分配，
// 不占用此固定布局）。
// early 栈（16 KiB，见 link.ld `. += 16K`）位于其后，但栈切换完成后即废弃，
// 可被 bump 回收。
unsafe extern "C" {
    static _free_base: u8;
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

    let free_base = (&raw const _free_base).addr();
    // 内核栈保留在 DRAM 顶部 [dram_end - KERNEL_STACK_SIZE, dram_end)，向下增长；
    // 其下再留一页 guard（[dram_end - KERNEL_STACK_SIZE - PAGE, dram_end - KERNEL_STACK_SIZE)）
    // 一并从 free 区剔除，避免 bump/frame 分配覆盖 guard/内核栈，且 guard 不映射
    // （manager::init 会 unmap），向下溢出即缺页。
    let free_end = dram_base + dram_size - KERNEL_STACK_SIZE - crate::memory::PAGE_SIZE;
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
