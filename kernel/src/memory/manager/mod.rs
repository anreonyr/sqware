// 虚拟内存管理 — Sv39 页表 + 地址空间 + 缺页 + ASID
//
// 子模块：
//   addr      — VirtAddr / PhysAddr
//   entry     — Sv39 PTE + PteFlags
//   fault     — 缺页处理
//   space     — AddressSpace、Region、内核地址空间初始化
//   table     — PageTable、页表遍历/映射（pub(crate)）
//   asid      — ASID 分配器
//
// 地址空间布局常量（TASK_STACK_*/USER_HEAP_*）与 satp/TLB 切换函数
// （switch_space / flush_asid）归本模块——它们描述「如何管理虚拟地址空间」，
// 与物理内存分配（crate::memory::allocator）解耦。

pub mod addr;
pub mod asid;
pub mod entry;
pub mod fault;
pub mod space;
pub(crate) mod table;

/// 页表操作错误 — `AddressSpace` pub 方法返回的错误类型。
///
/// 经 `pub use` 从 `pub(crate) mod table` 导出，使 pub API 签名中的类型
/// 可通过 `crate::memory::manager::MapError` 命名。bin crate 无外部消费者，
/// re-export 为「pub 签名类型可命名性」预留，故 allow(unused_imports)。
#[allow(unused_imports)]
pub use table::MapError;

/// 任务栈固定虚拟窗口基址 — 每任务栈映射到 `[TASK_STACK_BASE, +TASK_STACK_SIZE)`。
///
/// Sv39 低半区 L2 索引 3：内核仅映射 L2 0/1/2（MMIO / PCIe / DRAM）+ 高半区，
/// L2[3] 未映射 → `from_kernel` 浅克隆后各任务克隆里该条目无效 → 每个任务映射
/// 栈时各自分配私有 L1/L0，同一 VA 互不覆盖。守护页 = `[BASE-4K, BASE)` 保持
/// 未映射（栈溢出直接触发缺页：user → terminate / kernel → panic）。
///
/// 不变量：内核不得在内核空间映射 L2[3]（0xC000_0000..0x1_0000_0000）；
/// DRAM 必须 < 1 GiB（否则与 DRAM 恒等映射重叠，`space::init` 有断言）。
pub(crate) const TASK_STACK_BASE: usize = 0xC000_0000;

/// 每个任务栈的大小（字节）。
pub(crate) const TASK_STACK_SIZE: usize = 16384;

/// 用户堆固定基址 — map/unmap syscall 的堆区起点。
///
/// 用户空间布局：代码页 `0x10000`、Anonymous 示例 `0x7F00_0000`、
/// 堆 `[0x2000_0000, +64MiB)`、栈窗口 `0xC000_0000`——互不冲突。
/// 堆区从 [`USER_HEAP_BASE`] 单调分配（`heap_alloc` 游标），不回收。
pub(crate) const USER_HEAP_BASE: usize = 0x2000_0000;

/// 用户堆区大小（字节，64 MiB）。
pub(crate) const USER_HEAP_SIZE: usize = 0x40_0000;

/// Trap 入口 trampoline 页的固定虚拟地址（Sv39 最高页，L2[511]·L1[511]·L0[511]）。
///
/// 一页含 `__alltraps`（保存帧 + 切 satp）与 `__restore`（切回 + 恢复 + sret），
/// 内核空间与所有用户空间以同一 VA 映射**同一物理帧**（帧归内核所有，用户空间
/// 只映射不拥有）。`stvec` 指向此处：用户态任何 trap 从本 VA 取指——用户页表
/// 不含内核映射，trap 入口必须在两空间共同映射的页上。
///
/// 代码必须位置无关（只用 `li`/`ld`/`sd`/`csrrw`/`jr`/`sret`，禁用 `la`/`j`/`call`）：
/// PC 在 0xFFFF_FFFF_FFFF_F000，PC-relative 到内核低半区符号超出 ±2GiB 立即数范围。
pub(crate) const TRAMPOLINE: usize = 0xFFFF_FFFF_FFFF_F000;

/// 每空间 trap-context 页的固定虚拟地址（trampoline 下方一页，L2[511]·L1[511]·L0[510]）。
///
/// 用户空间把本任务的 [`crate::context::TrapContext`] 帧映射于此（S 态独占、无 U
/// 位）；内核空间映射自己的帧（供 boot/空闲/内核任务 trap 用）。`__alltraps` 在
/// 用户空间经此 VA 存帧；`__restore` 切回目标空间后经此 VA 恢复。内核在 KERNEL_SPACE
/// 经帧内 `self_pa` 字段（恒等映射）访问用户帧——无需重指向。
pub(crate) const TRAP_CONTEXT: usize = 0xFFFF_FFFF_FFFF_E000;

use crate::memory::arch::satp;

/// 构造 Sv39 satp token（写 satp 用）：`MODE | (ASID << 44) | PPN`。
///
/// 路线 1 后 satp 切换收敛到 trampoline 的 `__restore`（asm 直接写 satp + sfence），
/// 内核侧不再需要 `switch_space`——各处只需算出目标 token 传给 `__restore`。
#[inline(always)]
pub const fn satp_token(root_page_number: usize, asid: usize) -> usize {
    satp::make(satp::MODE_SV39, asid, root_page_number)
}

/// 刷新指定 ASID 的 TLB 条目（非全局）。
///
/// 发出 `sfence.vma zero, asid`（rs2 用通用寄存器传值：asid=0 时仅刷新
/// ASID 0 的非全局条目，asid≠0 时只刷新该 ASID）。页表修改（map/unmap/
/// protect）后按空间 ASID 调用，只使该地址空间的旧条目失效，其它任务的
/// TLB 热点保留。
///
/// # Safety
///
/// 调用者需确保刷新后页表仍然有效。
#[inline(always)]
pub unsafe fn flush_asid(asid: usize) {
    unsafe {
        core::arch::asm!("sfence.vma zero, {}", in(reg) asid);
    }
}
