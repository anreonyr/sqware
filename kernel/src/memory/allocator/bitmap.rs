// 位图分配器 — 通用编号空间连续区间分配器
//
// 在 [base, edge) 编号空间上按 unit 粒度分配连续区间（1 bit / unit，1 = 已分配）。
// 典型实例：用户堆窗口、任务栈窗口（per-Space，见 memory::manager::space）、
// ASID 空间（见 memory::manager::asid）——三类资源共用同一实现，释放即复用。
//
// 元数据全在外部（Vec<u64>，内核堆），**不写被管空间本身**——对未映射的 VA
// 窗口这是硬要求：侵入式 free-list（block.rs / frame.rs 风格）要把链表节点写进
// 空闲块，而空闲 VA 是未映射的，写入即内核缺页。
//
// 无内部锁：方法取 &mut self，调用方负责互斥。SpaceInner 内的实例随 Space 锁
// （RelLock）访问；ASID 全局实例由 asid.rs 的 SpinLock 保护（锁层级 L3）。

use alloc::alloc::AllocError;
use alloc::vec::Vec;

/// 位图分配器 — 在编号空间 [base, edge) 上按 unit 粒度分配连续区间。
///
/// 语义对齐 allocator 家族（block/frame）：`allocate` / `deallocate` + `AllocError`，
/// 但返回的是**地址/编号区间**而非可解引用的内存指针（本分配器管理未映射的
/// VA 或编号空间，不实现 [`core::alloc::Allocator`]）。
#[derive(Debug)]
pub(crate) struct BitmapAllocator {
    /// 空间基址（含）。
    base: usize,
    /// 空间上界（不含）。
    edge: usize,
    /// 分配粒度：VA 窗口 = PAGE_SIZE；ASID = 1。
    unit: usize,
    /// 位图（每 bit 一个 unit，1 = 已分配）；首次使用时按窗口尺寸惰性分配。
    bits: Vec<u64>,
}

impl BitmapAllocator {
    /// 构造（惰性：位图留待首次 allocate/deallocate 时按窗口尺寸分配，
    /// 使 const 静态实例可行，与 `ASID_ALLOCATOR` 的写法一致）。
    pub(crate) const fn new(base: usize, edge: usize, unit: usize) -> Self {
        Self {
            base,
            edge,
            unit,
            bits: Vec::new(),
        }
    }

    /// 分配 ≥ `size`（向上取整到 unit 倍数）的连续区间。
    ///
    /// first-fit：扫描位图找首个长度足够的全 0 连续段，置 1 后返回
    /// `(区间基址, 对齐后 size)`。`size == 0` 按 1 个 unit 处理。
    ///
    /// # Errors
    ///
    /// 空间耗尽或窗口内无足够连续空闲区间 → [`AllocError`]（调用方映射为
    /// 自己的错误，如 [`crate::memory::manager::MapError::OutOfMemory`]）。
    pub(crate) fn allocate(&mut self, size: usize) -> Result<(usize, usize), AllocError> {
        self.ensure();
        let units = size.div_ceil(self.unit).max(1);

        // first-fit：逐位扫描找连续 `units` 个 0（最坏 O(窗口位数)；
        // 教学简化，预留 next-fit 游标优化）。
        let mut run_start = 0usize;
        let mut run_len = 0usize;
        let mut found = false;
        for i in 0..self.units() {
            if self.bits[i / 64] & (1 << (i % 64)) == 0 {
                if run_len == 0 {
                    run_start = i;
                }
                run_len += 1;
                if run_len >= units {
                    found = true;
                    break;
                }
            } else {
                run_len = 0;
            }
        }
        if !found {
            return Err(AllocError);
        }

        for i in run_start..run_start + units {
            self.bits[i / 64] |= 1 << (i % 64);
        }
        Ok((self.base + run_start * self.unit, units * self.unit))
    }

    /// 精确匹配释放 `(addr, size)`：区间内 bit 全 1 → 清零 → `Ok(())`。
    ///
    /// # Errors
    ///
    /// 越界/非对齐/区间内含未分配 unit（从未分配或部分已释放）→ [`AllocError`]，
    /// 调用方按语义处理：堆路径返回 false（同旧块表精确匹配）、ASID 路径 panic
    /// （double-free 检测）。
    pub(crate) fn deallocate(&mut self, addr: usize, size: usize) -> Result<(), AllocError> {
        self.ensure();
        // 越界/非对齐：运行时检查（addr 可能来自 syscall 边界，不可只 debug_assert）
        if addr < self.base || addr + size > self.edge {
            return Err(AllocError);
        }
        if addr % self.unit != 0 || size % self.unit != 0 {
            return Err(AllocError);
        }
        let start = (addr - self.base) / self.unit;
        let units = size / self.unit;

        // 校验：区间必须全部在分配状态
        for i in start..start + units {
            if self.bits[i / 64] & (1 << (i % 64)) == 0 {
                return Err(AllocError);
            }
        }
        for i in start..start + units {
            self.bits[i / 64] &= !(1 << (i % 64));
        }
        Ok(())
    }

    /// 窗口内 unit 总数。
    fn units(&self) -> usize {
        (self.edge - self.base) / self.unit
    }

    /// 惰性尺寸：首次使用时按窗口尺寸分配位图（word 对齐向上取整）。
    fn ensure(&mut self) {
        if self.bits.is_empty() && self.units() > 0 {
            self.bits.resize(self.units().div_ceil(64), 0);
        }
    }
}
