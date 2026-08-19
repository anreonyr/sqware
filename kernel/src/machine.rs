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
    let n = get().hart;
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
    /// 时钟频率（DTB /cpus timebase-frequency，Hz；供 runtime::clock 注入）。
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

/// 注入机器信息（`main::probe` 调用，恰好一次）。
pub fn init(m: Machine) {
    MACHINE.set(m).unwrap();
}

/// 读取注入的机器信息（驱动按需调用）。
#[allow(dead_code)]
pub fn get() -> &'static Machine {
    MACHINE.get().expect("machine not initialized")
}
