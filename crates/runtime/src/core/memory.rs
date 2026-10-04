//! 程序侧内存分配与映射。

use env::{MemoryResult, PieToken, TeamId, VirtAddr};

/// 用户堆分配（按页取整、至少一页）。
///
/// # Errors
/// - `OoM`(-2)      段耗尽 / 物理帧耗尽
/// - `NoRegion`(-4) 本域没有 `user` 段（不变量破了）
pub fn allocate(size: usize) -> MemoryResult<usize> {
    env::memory::allocate(size).map(|va| va.get())
}

/// 用户堆释放（`(addr, size)` 必须精确匹配本段已分配的块）。
///
/// # Errors
/// - `Denied`(-1) 这一区间不在本任务那张簿记里
pub fn deallocate(addr: usize, size: usize) -> MemoryResult<()> {
    env::memory::deallocate(VirtAddr::new(addr), size)
}

/// 安装页映射（`at = 0` 窗口自选；`size`/`offset` 按页对齐；`flags` 是 R/W/X 位 1/2/3）。
///
/// # Errors
/// - `OoM`(-2)           窗口自选时段不足
/// - `NotAligned`(-3)    定点 `at` 未页对齐
/// - `AlreadyMapped`(-5) 定点 `at` 已被映射
pub fn map(
    team: TeamId,
    at: usize,
    size: usize,
    backing: PieToken,
    offset: usize,
    flags: u64,
) -> MemoryResult<usize> {
    env::memory::mmap(team, VirtAddr::new(at), size, backing, offset, flags).map(|va| va.get())
}

/// 释放 mmap / 声明区域（当前域：`TeamId(0)`）。
///
/// # Errors
/// - `Denied`(-1) 这一区间不是本段的已分配块
pub fn munmap(addr: usize, size: usize) -> MemoryResult<()> {
    env::memory::munmap(TeamId::new(0), VirtAddr::new(addr), size)
}
