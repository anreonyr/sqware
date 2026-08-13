// ASID 分配器 — Sv39 satp.ASID 字段（16 位）的独立分配
//
// ASID 0 保留给内核（KERNEL_SPACE / 空闲任务）；1..=65535 由任务地址空间
// 独占分配。每任务独立 ASID 让 TLB 按地址空间隔离：switch_space 写 satp 后
// 只 `sfence.vma zero, asid` 刷新本任务 ASID 的非全局条目，其余任务的 TLB
// 热点（栈顶 / 代码页）跨切换保留；页表修改（map/unmap/protect）同样按空间
// ASID 局部刷，不扰动其它任务。
//
// 生命周期与 AddressSpace 绑定：`from_kernel` 分配、`Drop` 释放。释放时先
// `sfence.vma zero, asid` 清该 ASID 的残留条目——ASID 会被后续任务复用，
// 残留条目会让新任务同 VA 命中旧任务的映射（物理页已归还/复用）。

use crate::lock::SpinLock;

/// ASID 位图（固定 1024 word = 65536 位，覆盖全部 16 位 ASID 空间）。
///
/// 位 i 置位 = ASID i 已分配。位 0（内核保留）恒置位。
struct AsidAllocator {
    bits: [u64; 1024],
}

impl AsidAllocator {
    const fn new() -> Self {
        let mut bits = [0u64; 1024];
        bits[0] = 1; // ASID 0（内核空间）保留
        Self { bits }
    }

    /// 分配一个空闲 ASID（≥1）；耗尽返回 None。
    fn allocate(&mut self) -> Option<usize> {
        for (wi, w) in self.bits.iter_mut().enumerate() {
            let free = !*w;
            if free != 0 {
                let bit = free.trailing_zeros() as usize;
                *w |= 1 << bit;
                return Some(wi * 64 + bit);
            }
        }
        None
    }

    /// 释放 ASID（位 0 不可释放；重复/未分配释放 panic）。
    fn deallocate(&mut self, asid: usize) {
        assert!(asid != 0 && asid < 65536, "asid: out of range {asid}");
        let (wi, bit) = (asid / 64, asid % 64);
        assert!(
            self.bits[wi] & (1 << bit) != 0,
            "asid: double-free or never-allocated {asid}"
        );
        self.bits[wi] &= !(1 << bit);
    }
}

static ASID_ALLOCATOR: SpinLock<AsidAllocator> = SpinLock::new(AsidAllocator::new());

/// 分配一个独立 ASID（1..=65535）。
///
/// 耗尽时 panic——65535 个并发任务远超系统能力（任务栈/地址空间内存也不够），
/// 静默退化为共享 ASID 0 会失去 TLB 隔离，宁 panic 不降级。
pub fn allocate() -> usize {
    ASID_ALLOCATOR
        .lock()
        .allocate()
        .expect("asid: 16-bit ASID space exhausted (65535 tasks)")
}

/// 释放 ASID 并刷新其 TLB 残留条目。
///
/// 释放后该 ASID 的旧条目（指向已归还/复用的物理页）必须失效——ASID 可能
/// 立即被新任务复用，同 VA 命中旧映射即数据错乱。G 位条目不受 ASID 过滤，
/// 但 G 条目来自共享内核映射、内容不变，残留无害。
pub fn deallocate(asid: usize) {
    ASID_ALLOCATOR.lock().deallocate(asid);
    // SAFETY: S-mode 下 sfence.vma 恒合法；rs2 用通用寄存器（非 x0）传 ASID，
    // 只刷新该 ASID 的非全局条目。
    unsafe {
        core::arch::asm!("sfence.vma zero, {}", in(reg) asid);
    }
}
