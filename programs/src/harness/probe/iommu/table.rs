//! 一级翻译页表：Sv39 三级，把一段 IOVA 映到真实物理页。
//!
//! PTE 位取自 Sv39：V[0] R[1] W[2] X[3] U[4] G[5] A[6] D[7]，PPN[53:10]。

use super::pages::Pages;

/// 每一级的索引位与项数。
const INDEX_BITS: usize = 9;
const PTES: usize = 1 << INDEX_BITS;
/// 非叶 PTE：只有有效位。
const BRANCH: u64 = 1 << 0;
/// 叶 PTE：有效 | 读 | 写 | **用户** | 已访问 | 已改。
///
/// 用户位是必须的：没有 process id 时（本景就是这一档），硬件把访问当成用户态，
/// 叶项缺 `U` 会被当成故障（QEMU `riscv_iommu_spa_fetch` 里那一条判据）。
const LEAF: u64 = (1 << 0) | (1 << 1) | (1 << 2) | (1 << 4) | (1 << 6) | (1 << 7);

/// 把 `iova` 那一页映到 `physical`（页对齐物理起址）。
///
/// 三级表页取 `pages` 里的 `root` / `middle` / `leaf` 三页；本手只写这一条路径。
pub fn map(pages: &Pages, root: usize, middle: usize, leaf: usize, iova: usize, physical: usize) {
    let index = |level: usize| (iova >> (12 + INDEX_BITS * level)) & (PTES - 1);
    write(
        pages.at(root, index(2) * 8),
        pte(BRANCH, pages.physical(middle)),
    );
    write(
        pages.at(middle, index(1) * 8),
        pte(BRANCH, pages.physical(leaf)),
    );
    write(pages.at(leaf, index(0) * 8), pte(LEAF, physical));
}

fn pte(flags: u64, physical: usize) -> u64 {
    flags | ((physical as u64 >> 12) << 10)
}

fn write(at: *mut u8, value: u64) {
    // SAFETY: 表页是本域刚取的可写页，偏移按 8 字节项对齐。
    unsafe { core::ptr::write_volatile(at as *mut u64, value) }
}
