// ASID 分配器 — satp.ASID 字段（16 位，64 位各模式同宽）的独立分配。
//
// ASID 0 保留；1..=65535 由任务地址空间独占。每任务独立 ASID 让 TLB 按地址
// 空间隔离：只刷本任务 ASID 的非全局条目，其余任务热点跨切换保留。
//
// 释放时**先跨核清退、再还位图**：还位图之后 ASID 立即可被复用，任何核上的
// 残留条目都会让新空间同 VA 命中旧映射——顺序不可换。
//
// 实现：位图分配器（[`BitmapAllocator`]）全局实例，base = 1、unit = 1。
use crate::lock::{Level, SpinLock};
use crate::memory::allocator::bitmap::BitmapAllocator;
use crate::memory::manager::evict::{self, Deaf};
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
/// 清退该 ASID 的全系统 TLB 残留后归还位图。double-free/未分配 panic。
///
/// # Errors
///
/// [`Deaf`] = 某核未在耐心内到齐；此时位图未动（ASID 不会被复用）。
pub fn deallocate(asid: usize) -> Result<(), Deaf> {
    evict::evict(asid)?;
    ASID_ALLOCATOR
        .lock()
        .deallocate(asid, 1)
        .expect("asid: double-free or never-allocated");
    Ok(())
}
