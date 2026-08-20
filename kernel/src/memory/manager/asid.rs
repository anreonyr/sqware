// ASID 分配器 — Sv39 satp.ASID 字段（16 位）的独立分配
//
// ASID 0 保留给内核（内核团队 / 空闲任务）；1..=65535 由任务地址空间
// 独占分配。每任务独立 ASID 让 TLB 按地址空间隔离：switch_space 写 satp 后
// 只 `sfence.vma zero, asid` 刷新本任务 ASID 的非全局条目，其余任务的 TLB
// 热点（栈顶 / 代码页）跨切换保留；页表修改（map/unmap/protect）同样按空间
// ASID 局部刷，不扰动其它任务。
//
// 生命周期与 Space 绑定：`SpaceBuilder::user()` 分配、`Drop` 释放。释放时先
// `sfence.vma zero, asid` 清该 ASID 的残留条目——ASID 会被后续任务复用，
// 残留条目会让新任务同 VA 命中旧任务的映射（物理页已归还/复用）。
//
// 实现：位图分配器（[`BitmapAllocator`]）的全局实例——ASID 0 内核保留不入
// 空间，故 base = 1；unit = 1（每 ASID 一位），1..=65535 全覆盖。与 VA 窗口
// （space.rs 的堆/栈实例）共用同一通用分配器，释放即复用。
use crate::lock::SpinLock;
use crate::memory::allocator::bitmap::BitmapAllocator;
/// ASID 0 内核保留不入空间 → base = 1；1..=65535 全覆盖，unit = 1。
/// 位图（1024 word = 65536 位）首次 allocate 时经 `ensure` 惰性分配。
static ASID_ALLOCATOR: SpinLock<BitmapAllocator> = SpinLock::new(BitmapAllocator::new(1, 65536, 1));
/// 分配一个独立 ASID（1..=65535）。
///
/// 耗尽时 panic——65535 个并发任务远超系统能力（任务栈/地址空间内存也不够），
/// 静默退化为共享 ASID 0 会失去 TLB 隔离，宁 panic 不降级。
pub fn allocate() -> usize {
    let (asid, _) = ASID_ALLOCATOR
        .lock()
        .allocate(1)
        .expect("asid: 16-bit ASID space exhausted (65535 tasks)");
    asid
}
/// 释放 ASID 并刷新其 TLB 残留条目。
///
/// 释放后该 ASID 的旧条目（指向已归还/复用的物理页）必须失效——ASID 可能
/// 立即被新任务复用，同 VA 命中旧映射即数据错乱。G 位条目不受 ASID 过滤，
/// 但 G 条目来自共享内核映射、内容不变，残留无害。double-free/未分配 panic。
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
