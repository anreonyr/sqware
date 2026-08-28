// 机器设备信息 — 启动时从设备树一次性解析出的纯值（无引用、无 fdt 依赖）

use core::ops;
use core::sync::atomic::{AtomicUsize, Ordering};

use crate::layout::{HART_FRAME_BASE, ROOT_STACK_SIZE};
use crate::lock::OnceLock;
use crate::memory::PAGE_SIZE;
use crate::memory::manager::addr::VirtAddr;

/// 半开物理区间 `[base, end)` — 内存池 / MMIO 设备区域通用。
///
/// 长度用 `end - base` 计算，不单独存 size。
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

/// 可寻址的 per-hart 槽数上限（编译期常量 = hart 帧区 VA 窗口宽度与
/// per-hart trap 栈窗口宽度，4096 段 = 高位虚拟地址。虚拟地址免费，故放得慷慨：
/// **窗口宽度与核数解耦**——实际启用核数由 DTB 运行时决定（[`hart_count`]），
/// 本常量只是页表槽位的编译期防呆上限（超过即 panic，不静默截断）。
///
/// 定位即两层制约：本常量 = **VA 布局表达上限**（窗口撑不下即 panic）；
/// **物理养活上限**由 boot 期 per-hart 开销校验把握（trap::init，内存制约核数
/// 的运行时落点）。两者之上以 DTB 上报核数为运行真值。
///
/// 注意：这不是 SBI 协议边界。SBI 的 `sbi_send_ipi` 每次调用至多寻址
/// XLEN(=64) 个 hart（掩码寄存器位宽），超过需按 64 核一组多次调用；
/// 协议对总核数不设上限。
pub const MAX_HART_SLOTS: usize = 4096;

/// 已启动的 hart 集合（进程级进度记录；无功能读者，保留为诊断信息）。
static STARTED_HARTS: AtomicUsize = AtomicUsize::new(1);

/// 记录某 hart 已启动（HSM `hart_start` 成功后调用）。
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
/// hartid 经 `tp` 指向的 [`PerHart`] 读取：`tp` = 本 hart 的 `PerHart` 指针
/// （入口/陷阱重建维护，见 `main.rs`/`_boot_entry`/`establish_tp`）。
#[inline]
pub fn hart_id() -> usize {
    let id: usize;
    // SAFETY: 读 tp 指向的 PerHart.id（内核态 tp 恒为本 hart PerHart 指针，无副作用）。
    unsafe {
        core::arch::asm!(
            "ld {0}, 0(tp)",
            out(reg) id,
            options(nomem, nostack, preserves_flags),
        );
    }
    id
}

/// per-hart 上下文块——内核态 tp 指向本结构（替代旧「tp 存裸 hartid」约定）。
///
/// 汇编消费端（`__strap`/`__restore` 定位本 hart 帧 VA、`reap` 读 hartid）
/// 按本结构裸偏移访问，布局由编译期断言锁死（`offset_of` 检查）。
///
/// 为什么是常量数组而非运行时分配：boot 汇编（`_start`/`_boot_entry`）在
/// 机器信息注入前就要设 tp，数组必须是链接期已知符号；槽数= MAX_HART_SLOTS
/// （VA 布局表达上限）；物理支撑由 boot 期 per-hart 开销校验保证（见 trap::init）。
#[repr(C)]
#[derive(Clone, Copy)]
pub struct PerHart {
    /// 本 hart 编号（offset 0x00；`hart_id()` 读这里）。
    pub id: usize,
    /// 本 hart 帧 VA（offset 0x08；`HART_FRAME_BASE + id·PAGE`，`__strap` 帧定位）。
    pub frame: VirtAddr,
}

impl PerHart {
    const fn at(id: usize) -> Self {
        Self {
            id,
            // 布局常量纯算术：帧区基址 + 槽位偏移（同 layout.rs 推导）。
            frame: VirtAddr::wrap(HART_FRAME_BASE.as_usize() + id * PAGE_SIZE),
        }
    }
}

/// per-hart 上下文数组（4096 槽 × 16 B = 64 KiB 静态数据）。
///
/// `no_mangle`：boot 汇编经 `la t0, PER_HART` PC 相对定位（恒等映射，Bare 期
/// PC 相对即物理地址）。槽位随 `MAX_HART_SLOTS`（VA 布局表达上限）。
#[unsafe(no_mangle)]
static PER_HART: [PerHart; MAX_HART_SLOTS] = {
    let mut a = [PerHart::at(0); MAX_HART_SLOTS];
    let mut i = 1;
    while i < MAX_HART_SLOTS {
        a[i] = PerHart::at(i);
        i += 1;
    }
    a
};

/// 指定 hart 的 PerHart 指针（`tp` 装载值 / 帧 TP 装配共用）。
#[inline]
pub fn per_hart_ptr(id: usize) -> usize {
    debug_assert!(
        id < MAX_HART_SLOTS,
        "per_hart_ptr: id {id} beyond MAX_HART_SLOTS"
    );
    core::ptr::addr_of!(PER_HART[id]) as usize
}

/// 编译期断言：PerHart 布局即 ABI（`__strap`/`__restore`/`reap` 汇编按偏移访问）。
const _: () = {
    assert!(core::mem::offset_of!(PerHart, id) == 0x00);
    assert!(core::mem::offset_of!(PerHart, frame) == 0x08);
    assert!(core::mem::size_of::<PerHart>() == 16);
};

/// 启动时从 DTB 解析出的机器设备信息（纯值，Copy，可安全存入 static）。
#[derive(Clone, Copy, Debug)]
pub struct Machine {
    /// CPU 核数。
    pub hart: usize,
    /// 时钟频率（DTB /cpus timebase-frequency，Hz）。
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

    let free_base = root_stack_edge();
    // ROOT 栈位于镜像内（guard + 栈区），整个空闲区
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

/// DRAM 物理上界（exclusive，恒等区可直读区间的上界）。
/// 机器信息未注入（`machine::init` 前）→ None，调用方自行退回保守值。
/// 取 None 而非 panic：崩溃现场绝不能再 panic。
pub(crate) fn dram_end() -> Option<usize> {
    MACHINE.get().map(|m| m.dram.range().end)
}

/// ROOT 栈 canary 值（写在栈底；boot 移交审核 + panic 归巢复读共用）。
pub(crate) const ROOT_STACK_CANARY: usize = 0x600D_CAFE_51A7_0D1E;

/// 镜像结束地址（链接脚本 `_kernel_edge`）——栈与 free 区布局的唯一基准。
pub(crate) fn kernel_edge() -> usize {
    (&raw const _kernel_edge).addr()
}
unsafe extern "C" {
    /// 内核镜像结束锚点（见 link.ld）。栈区与 free 区都从它推导。
    static _kernel_edge: u8;
}

/// ROOT 栈区偏移（栈大小）——`_start` 汇编加载它算出栈顶 sp。
/// no_mangle 暴露为符号，global_asm `la t0,_stack; ld t0,0(t0)` 读取，
/// 栈大小单一来源（此处由 Rust 常量推导），链接脚本不写栈布局。
#[unsafe(no_mangle)]
static _stack: usize = ROOT_STACK_SIZE;

#[unsafe(no_mangle)]
static _canary: usize = ROOT_STACK_CANARY;

/// ROOT 栈底地址（canary 所在）。
pub(crate) fn root_stack_base() -> usize {
    kernel_edge()
}

/// ROOT 栈顶地址（= free 区起点；panic `home` 归巢落点）。
pub(crate) fn root_stack_edge() -> usize {
    kernel_edge() + ROOT_STACK_SIZE
}

/// 读取 DTB `/cpus` 的 timebase-frequency（Hz）；缺失/非法长度返回 0。
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
