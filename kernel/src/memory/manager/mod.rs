// 虚拟内存管理 — Sv39 页表 + 地址空间 + 缺页 + ASID
//
// 子模块：
//   addr      — VirtAddr / PhysAddr
//   entry     — Sv39 PTE + PteFlags
//   fault     — 缺页处理
//   table     — PageTable、页表遍历/映射（pub(crate)）
//   asid      — ASID 分配器
//
// satp/TLB 切换函数（flush_asid）与错误/结果类型（MapError/MapResult）归本模块，
// 描述「如何管理虚拟地址空间」这一原语层，与物理内存分配（crate::memory::allocator）解耦。
// 地址空间**类型/实例**（Space/Team 容器）、布局常量与内核地址空间的构建/装配
// （原 memory::manager::init）已全部收编进 work::unit（unit::space + unit::init）——
// 本模块对 work 零依赖、亦不上穿 runtime，只供应原语与错误接缝。

pub mod addr;
pub mod asid;
pub mod entry;
pub mod fault;
pub mod table;

/// 页表操作错误 — `Space` pub 方法返回的错误类型。
///
/// 经 `pub use` 从 `pub(crate) mod table` 导出，使 pub API 签名中的类型
/// 可通过 `crate::memory::manager::MapError` 命名。bin crate 无外部消费者，
/// re-export 为「pub 签名类型可命名性」预留，故 allow(unused_imports)。
pub use table::MapError;

/// 刷新指定 ASID 的 TLB 条目（非全局）。
///
/// 本地 `sfence.vma zero, asid`（rs2 用通用寄存器传值：asid=0 时全刷
/// 含全局条目，asid≠0 时只刷新该 ASID）。页表修改（map/unmap/protect）后
/// 按空间 ASID 调用。
///
/// **无需远程刷（RFNC）**：每次 satp 切换（`__alltraps`/`__restore`）都已全刷
/// 本地 TLB（rs1=rs2=x0），跨核不会残留陈旧条目——远程核只在切换 satp 到自己
/// 空间时接触其页表，而切换本身即全刷；内核空间 post-boot 无 map/unmap。休眠
/// 核（WFI）醒来后同样经 satp 切换全刷，不依赖远程 fence。
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
