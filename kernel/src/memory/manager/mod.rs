// 虚拟内存管理 — 随模式的多级页表 + 地址空间 + 缺页 + ASID。
//
//   addr   — VirtAddr / PhysAddr
//   entry  — PTE + PteFlags（Sv39/48/57 同格式）
//   fault  — 缺页处理
//   table  — PageTable、页表遍历/映射（pub(crate)）
//   asid   — ASID 全生命周期（编号分配 + 宿住登记 + TLB 清退）
//   mode   — 运行模式探测

pub mod addr;
pub mod asid;
pub mod entry;
pub mod fault;
pub mod mode;
pub mod table;

pub use table::MapError;

use asid::Asid;

/// 刷新指定 ASID 的 TLB 条目（非全局）：`sfence.vma zero, asid`。**asid 经通用
/// 寄存器传入 ⇒ 任何取值都按 ASID 匹配、永不失效全局（G=1）条目**——asid=0 亦然
/// （"asid=0 连全局一起刷"只对 `rs2=x0` 的写法成立）。页表修改后按空间 ASID 调用。
///
/// 照实记：`flush_asid(Asid::kernel())`（内核空间，`space/outer.rs`，调用点
/// `unit/mod.rs`）本意是**连全局条目一起全刷**；今日的 asm 不覆盖 G=1 的内核映射。
/// 尚未见由此产生的故障，但账要这么记。
///
/// # Safety
///
/// 调用者需确保刷新后页表仍然有效。
#[inline(always)]
pub unsafe fn flush_asid(asid: Asid) {
    unsafe {
        core::arch::asm!("sfence.vma zero, {}", in(reg) asid.get());
    }
}
