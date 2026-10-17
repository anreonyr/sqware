//! 一级翻译页表：Sv39 三级，把一段 IOVA 映到真实物理页。
//!
//! PTE 位取自 Sv39：V[0] R[1] W[2] X[3] U[4] G[5] A[6] D[7]，PPN[53:10]。

use env::PAGE_SIZE;

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

/// 把 `count` 页从 `iova_base` 起依次映到 `pages` 里 `first..first+count` 那几页。
///
/// 三级表页取 `pages` 里的 `root` / `middle` / `leaf` 三页，本手只写这一条路径——
/// 故这一批 IOVA 必须落在**同一个 2MiB** 里（调用方那一侧只映一段窗口）。
/// 超出方向或越窗当场拒，不静默少映。
pub fn map_pages(
    pages: &Pages,
    root: usize,
    middle: usize,
    leaf: usize,
    iova_base: usize,
    first: usize,
    count: usize,
) -> Result<(), &'static str> {
    if count == 0 || !iova_base.is_multiple_of(PAGE_SIZE) {
        return Err("iommu: iova window");
    }
    let index = |iova: usize, level: usize| (iova >> (12 + INDEX_BITS * level)) & (PTES - 1);
    let last = iova_base + (count - 1) * PAGE_SIZE;
    let shared = index(iova_base, 2) == index(last, 2) && index(iova_base, 1) == index(last, 1);
    if !shared || first + count > pages.count() {
        return Err("iommu: iova window");
    }
    write(
        pages.at(root, index(iova_base, 2) * 8),
        pte(BRANCH, pages.physical(middle)),
    );
    write(
        pages.at(middle, index(iova_base, 1) * 8),
        pte(BRANCH, pages.physical(leaf)),
    );
    for at in 0..count {
        let iova = iova_base + at * PAGE_SIZE;
        write(
            pages.at(leaf, index(iova, 0) * 8),
            pte(LEAF, pages.physical(first + at)),
        );
    }
    Ok(())
}

fn pte(flags: u64, physical: usize) -> u64 {
    flags | ((physical as u64 >> 12) << 10)
}

fn write(at: *mut u8, value: u64) {
    // SAFETY: 表页是本域刚取的可写页，偏移按 8 字节项对齐。
    unsafe { core::ptr::write_volatile(at as *mut u64, value) }
}
