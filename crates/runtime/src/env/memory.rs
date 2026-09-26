//! Memory 域：`MemoryCall::*` 转发。
//!
//! **每格一个精确签名的入口**（`env::memory::*`，由 `#[derive(Envcall)]` 生成）：本层只做
//! "调用方口径 → 内核口径"的那点转换（按页取整、`Option` → 哨兵），错类型是**这一域的
//! 词汇**（`MemoryFail`）——`Allocate` 答得出 `OoM` 与 `NoRegion`，`Deallocate` 只答
//! `Denied`。从前这里拿 `MemoryCallRet` 再 `match` 一趟、还带一条 `unreachable!`：那一趟
//! 是"一个 `call()` 对整张 `Ret` 联合负责"逼出来的，现在由宏按格生成，退掉了。

use env::{MemoryResult, VirtAddr};

use crate::PAGE_SIZE;

/// 用户堆分配（按页取整、至少一页）。
///
/// # Errors
/// - `OoM`(-2)      段耗尽 / 物理帧耗尽
/// - `NoRegion`(-4) 本域没有 `user` 段（不变量破了）
pub fn allocate(size: usize) -> MemoryResult<usize> {
    let size = size.max(1).next_multiple_of(PAGE_SIZE);
    env::memory::allocate(size).map(|va| va.get())
}

/// 用户堆释放（`(addr, size)` 必须精确匹配本段已分配的块）。
///
/// # Errors
/// - `Denied`(-1) 这一区间不在本任务那张簿记里
pub fn deallocate(addr: usize, size: usize) -> MemoryResult<()> {
    let size = size.max(1).next_multiple_of(PAGE_SIZE);
    env::memory::deallocate(VirtAddr::new(addr), size)
}

/// `at = None` 走窗口自选，`Some(addr)` 走固定地址。
///
/// # Errors
/// - `OoM`(-2)           窗口自选时段不足
/// - `NotAligned`(-3)    定点 `addr` 未页对齐
/// - `AlreadyMapped`(-5) 定点 `addr` 已被映射
pub fn mmap(size: usize, at: Option<usize>) -> MemoryResult<usize> {
    let size = size.max(1).next_multiple_of(PAGE_SIZE);
    env::memory::mmap(size, VirtAddr::new(at.unwrap_or(0))).map(|va| va.get())
}

/// 释放 mmap / 声明区域。
///
/// # Errors
/// - `Denied`(-1) 这一区间不是本段的已分配块
pub fn munmap(addr: usize, size: usize) -> MemoryResult<()> {
    let size = size.max(1).next_multiple_of(PAGE_SIZE);
    env::memory::munmap(VirtAddr::new(addr), size)
}

/// 修改映射区域保护标志。
///
/// # Errors
/// - `Denied`(-1) 标志位非法 / 覆盖不足 / 借入页不许加宽
pub fn mprotect(addr: usize, size: usize, flags: u64) -> MemoryResult<()> {
    let size = size.max(1).next_multiple_of(PAGE_SIZE);
    env::memory::mprotect(VirtAddr::new(addr), size, flags)
}
