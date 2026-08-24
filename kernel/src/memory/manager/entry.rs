// 页表项 (Page Table Entry) — 64-bit PTE 类型定义与操作（Sv39/Sv48/Sv57 同格式）
//
// 每个条目为 8 字节，位布局：
//   0:9   — 标志位 (V, R, W, X, U, G, A, D, RSW*2)
//   10:53 — PPN (物理页号, 44 bits, 对应 56-bit 物理地址的 12:55)
//   54:63 — 保留 (必须为零)

use bitflags::bitflags;
use core::fmt;

bitflags! {
    /// PTE 标志位 (bits 0-9)
    #[derive(Debug, Clone, Copy)]
    pub struct PteFlags: u64 {
        /// Valid — PTE 有效
        const V = 1 << 0;
        /// Read — 可读
        const R = 1 << 1;
        /// Write — 可写
        const W = 1 << 2;
        /// Execute — 可执行
        const X = 1 << 3;
        /// User — 用户态可访问
        const U = 1 << 4;
        /// Global — 全局映射（不随 sfence.vma ASID 刷新）
        const G = 1 << 5;
        /// Accessed — 已被访问（硬件会置位）
        const A = 1 << 6;
        /// Dirty — 已被写入（硬件会置位）
        const D = 1 << 7;
    }
}

/// 页表项
#[repr(transparent)]
#[derive(Clone, Copy, Default)]
pub struct PageTableEntry {
    bits: u64,
}

impl PageTableEntry {
    const FLAGS_MASK: u64 = 0x3FF; // bits 0-9
    const PPN_SHIFT: usize = 10;

    // ── 查询 ────────────────────────────────────────────────

    /// PTE 是否有效（V=1）
    #[inline(always)]
    pub fn is_valid(self) -> bool {
        self.flags().contains(PteFlags::V)
    }

    /// 是否为叶子节点（R|W|X 中任一位被设置）
    #[inline(always)]
    pub fn is_leaf(self) -> bool {
        self.flags()
            .intersects(PteFlags::R | PteFlags::W | PteFlags::X)
    }

    /// 是否为 branch 节点（V=1 且 R=W=X=0，指向下一级页表）
    #[inline(always)]
    pub fn is_branch(self) -> bool {
        self.is_valid() && !self.is_leaf()
    }

    /// 提取 PPN（44 位，左移 12 得物理地址）
    #[inline(always)]
    pub fn ppn(self) -> u64 {
        self.bits >> Self::PPN_SHIFT
    }

    /// 提取物理地址（PPN << 12）
    #[inline(always)]
    pub fn paddr(self) -> u64 {
        self.ppn() << 12
    }

    /// 提取标志位
    #[inline(always)]
    pub fn flags(self) -> PteFlags {
        PteFlags::from_bits_truncate(self.bits & Self::FLAGS_MASK)
    }

    // ── 修改 ────────────────────────────────────────────────

    /// 设置 PPN 和标志位
    #[inline(always)]
    pub fn set(&mut self, ppn: u64, flags: PteFlags) {
        self.bits = (ppn << Self::PPN_SHIFT) | flags.bits();
    }

    /// 设置标志位（保留 PPN 不变）
    #[inline(always)]
    pub fn set_flags(&mut self, flags: PteFlags) {
        self.bits = (self.bits & !Self::FLAGS_MASK) | flags.bits();
    }

    /// 清除 PTE（设为无效）
    #[inline(always)]
    pub fn clear(&mut self) {
        self.bits = 0;
    }
}

impl fmt::Debug for PageTableEntry {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if !self.is_valid() {
            write!(f, "PTE(invalid)")
        } else if self.is_branch() {
            write!(f, "PTE(branch → {:#x})", self.paddr())
        } else {
            write!(
                f,
                "PTE({:#x}, ppn={:#x}, flags={:?})",
                self.bits,
                self.ppn(),
                self.flags()
            )
        }
    }
}
