// 机器设备信息 — 启动时从设备树一次性解析出的纯值（无引用、无 fdt 依赖）
//
// 设计原则：DTB 解析只发生在 main::probe 一次，产出 Copy 的纯值注入；任何模块
// 都不直接依赖 fdt crate。模块按需取标量字段——自包含模块（如 memory）只收
// Region，不反向依赖整个 Machine。读路径经 OnceLock，仅一次 AtomicBool::load，
// 无锁。
//
// 与 memory 的关系：本文件只定义纯值类型 + 注册表（不含任何 DTB 探测），
// memory 仅依赖这里的 `Region`，不重引入旧的 platform 耦合。

use core::ops;
use core::sync::atomic::{AtomicUsize, Ordering};

use crate::lock::OnceLock;

/// 半开物理区间 `[base, end)` — 内存池 / MMIO 设备区域通用。
///
/// 长度用 `end - base` 计算，不单独存 size。与 memory 的 debug 越界检查
/// 口径一致（`base..end`）。
#[derive(Clone, Copy, Debug)]
pub struct Region {
    pub base: usize,
    pub size: usize,
}

impl Region {
    pub fn new(base: usize, size: usize) -> Self {
        Self { base, size }
    }
    pub fn range(&self) -> ops::Range<usize> {
        self.base..self.base + self.size
    }
}

/// 可寻址的 per-hart 槽数上限（编译期常量 = 内核帧区 VA 窗口宽度，4096 页 =
/// 16 MiB 高位虚拟地址）。虚拟地址免费，故放得慷慨：**窗口宽度与核数解耦**——
/// 实际启用核数由 DTB 运行时决定（[`hart_count`]），本常量只是页表槽位的
/// 编译期防呆上限（超过即 panic，不静默截断）。
///
/// 注意：这不是 SBI 协议边界。SBI 的 `sbi_send_ipi` 每次调用至多寻址
/// XLEN(=64) 个 hart（掩码寄存器位宽），超过需按 64 核一组多次调用
/// （tie::wake_all 已按此循环）；协议对总核数不设上限。
pub const MAX_HART_SLOTS: usize = 4096;

/// 已启动的 hart 集合（进程级进度记录；无功能读者，保留为诊断信息）。
static STARTED_HARTS: AtomicUsize = AtomicUsize::new(1);

/// 记录某 hart 已启动（HSM `hart_start` 成功后由 hart 0 调用）。
pub fn mark_hart_started(hart: usize) {
    debug_assert!(
        hart < MAX_HART_SLOTS,
        "hart id {hart} beyond MAX_HART_SLOTS {MAX_HART_SLOTS}"
    );
    STARTED_HARTS.fetch_max(hart + 1, Ordering::Relaxed);
}

/// 实际活跃核数 = DTB 上报核数（上限 = VA 窗口槽数 MAX_HART_SLOTS）。
///
/// **动态获取**：核数完全由 DTB 决定（`Machine.hart`，运行时注入）。
/// per-hart 结构（调度器数组 / trap 栈 / 内核帧 / 帧 PA 表）都按此值定尺寸。
pub fn hart_count() -> usize {
    let n = info().hart;
    assert!(
        n <= MAX_HART_SLOTS,
        "DTB reports {n} harts, at most {MAX_HART_SLOTS} VA slots"
    );
    n
}

/// 当前 hart id（**执行本代码的核**）——与 `Machine::hart`（总核数）不同。
///
/// S-mode 读不到 M-mode 专属 CSR `mhartid`（读它触发 illegal instruction），故
/// hartid 在入口处存入 `tp`。本内核不使用 TLS，`tp` 恒为入口值，不被任何子例程改写。
#[inline]
pub fn hart_id() -> usize {
    let id: usize;
    // SAFETY: 读取线程指针寄存器（入口处由 `_start` 写入 hartid），纯读、无副作用。
    unsafe {
        core::arch::asm!("mv {0}, tp", out(reg) id, options(nomem, nostack, preserves_flags));
    }
    id
}

/// 启动时从 DTB 解析出的机器设备信息（纯值，Copy，可安全存入 static）。
#[derive(Clone, Copy, Debug)]
pub struct Machine {
    /// CPU 核数。
    pub hart: usize,
    /// 时钟频率（DTB /cpus timebase-frequency，Hz；供 runtime::time 注入）。
    pub hertz: usize,
    /// 物理内存范围
    pub dram: Region,
    /// 物理内存空闲区
    pub free: Region,
    #[allow(dead_code)]
    pub uart: Region,
    #[allow(dead_code)]
    pub plic: Region,
    #[allow(dead_code)]
    pub clint: Region,
}

static MACHINE: OnceLock<Machine> = OnceLock::new();

/// 注入机器信息
pub fn init(dtp: usize) {
    let fdt = unsafe { fdt::Fdt::from_ptr(dtp as *const u8) }.expect("invalid device tree blob");

    let hart = fdt.cpus().count();

    let mem = fdt
        .memory()
        .regions()
        .next()
        .expect("device tree has no /memory node");
    let dram_base = mem.starting_address.addr();
    let dram_size = mem.size.unwrap_or(0);
    let hertz = hertz(&fdt);

    let free_base = kernel_stack_edge();
    // 主内核栈位于镜像内（guard + 主栈区），整个空闲区
    // （free_base = 栈顶 .. dram_end）均可分配。
    let free_end = dram_base + dram_size;
    let free_size = free_end - free_base;

    MACHINE
        .set(Machine {
            dram: Region::new(dram_base, dram_size),
            free: Region::new(free_base, free_size),
            hart,
            hertz,
            uart: Region::new(0, 0),
            plic: Region::new(0, 0),
            clint: Region::new(0, 0),
        })
        .unwrap()
}

/// 读取注入的机器信息（驱动按需调用）。
pub fn info() -> &'static Machine {
    MACHINE.get().expect("machine not initialized")
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

#[unsafe(no_mangle)]
static _canary: usize = KERNEL_STACK_CANARY;

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
fn hertz(fdt: &fdt::Fdt) -> usize {
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
