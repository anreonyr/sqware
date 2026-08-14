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

use crate::lock::OnceLock;

/// 半开物理区间 `[base, end)` — 内存池 / MMIO 设备区域通用。
///
/// 长度用 `end - base` 计算，不单独存 size。与 memory 的 debug 越界检查
/// 口径一致（`base..end`）。
#[derive(Clone, Copy)]
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

/// 启动时从 DTB 解析出的机器设备信息（纯值，Copy，可安全存入 static）。
#[derive(Clone, Copy)]
pub struct Machine {
    /// CPU 核数。
    pub hart: usize,
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
    let _ = MACHINE.set(m);
}

/// 读取注入的机器信息（驱动按需调用）。
#[allow(dead_code)]
pub fn get() -> &'static Machine {
    MACHINE.get().expect("machine not initialized")
}
