// ASID 分配器 — satp.ASID 字段（16 位，64 位各模式同宽）的独立分配。
//
// ASID 0 保留；1..=65535 由任务地址空间独占。每任务独立 ASID 让 TLB 按地址
// 空间隔离：只刷本任务 ASID 的非全局条目，其余任务热点跨切换保留。
//
// 释放时先 `sfence.vma zero, asid` 清残留条目——ASID 会被复用，残留条目会让
// 新任务同 VA 命中旧映射。
//
// 实现：位图分配器（[`BitmapAllocator`]）全局实例，base = 1、unit = 1。
use crate::lock::{Level, SpinLock};
use crate::memory::allocator::bitmap::BitmapAllocator;
static ASID_ALLOCATOR: SpinLock<BitmapAllocator> =
    SpinLock::new_level(Level::Asid, BitmapAllocator::new(1, 65536, 1));
/// 分配一个独立 ASID（1..=65535）。耗尽时 panic。
pub fn allocate() -> usize {
    let (asid, _) = ASID_ALLOCATOR
        .lock()
        .allocate(1)
        .expect("asid: 16-bit ASID space exhausted (65535 tasks)");
    asid
}
/// 释放 ASID 并刷新其 TLB 残留条目。double-free/未分配 panic。
pub fn deallocate(asid: usize) {
    ASID_ALLOCATOR
        .lock()
        .deallocate(asid, 1)
        .expect("asid: double-free or never-allocated");
    // SAFETY: S-mode 下 sfence.vma 恒合法；rs2 用通用寄存器（非 x0）传 ASID，
    // 只刷新该 ASID 的非全局条目。
    unsafe {
        core::arch::asm!("sfence.vma zero, {}", in(reg) asid);
    }
}
