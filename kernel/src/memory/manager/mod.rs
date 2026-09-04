// 虚拟内存管理 — 随模式的多级页表 + 地址空间 + 缺页 + ASID。
//
//   addr   — VirtAddr / PhysAddr
//   entry  — PTE + PteFlags（Sv39/48/57 同格式）
//   evict  — 跨核 TLB 清退（租约册 + 清退协议）
//   fault  — 缺页处理
//   table  — PageTable、页表遍历/映射（pub(crate)）
//   asid   — ASID 分配器

pub mod addr;
pub mod asid;
pub mod entry;
pub mod evict;
pub mod fault;
pub mod mode;
pub mod table;

/// 页表操作错误。
pub use table::MapError;

/// 刷新指定 ASID 的 TLB 条目（非全局）：`sfence.vma zero, asid`（asid=0 时全刷
/// 含全局条目，asid≠0 时只刷新该 ASID）。页表修改后按空间 ASID 调用。
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

/// 整表刷新（全 ASID）：`sfence.vma`。清退协议的应答点用它兑现「世代递增 ⟺
/// 已整表刷过」（见 `evict` 不变量 1）。
///
/// # Safety
///
/// 调用者需确保刷新后页表仍然有效。
#[inline(always)]
pub unsafe fn flush_all() {
    unsafe {
        core::arch::asm!("sfence.vma");
    }
}
