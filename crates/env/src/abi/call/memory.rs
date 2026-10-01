//! call::memory — **Memory 域（class 2：地址空间与页）**：调用表（[`MemoryCall`]）与失败词汇（[`MemoryFail`]）。

use mold::{Envcall, Fail};
use crate::wire::{VirtAddr};

/// Memory 域（class 2：用户堆与映射）的失败词汇。
#[derive(Fail)]
pub enum MemoryFail {
    /// 这一区间不在本任务那张簿记里 / `flags` 非法。
    Denied = -1,
    /// 段耗尽 / 物理帧耗尽 / 映射表备不下。
    OoM = -2,
    /// 地址未按页对齐（定点 `mmap` / `mprotect`）。
    NotAligned = -3,
    /// 这一段不在任何已声明的映射里。
    NoRegion = -4,
    /// 该 VA 已被映射。
    AlreadyMapped = -5,
    /// 借入页（别人的所有权）不许加宽。
    WidenDenied = -6,
}

/// `MemoryFail` 的结果别名。
pub type MemoryResult<T> = Result<T, erra::Error<MemoryFail>>;

/// 内存调用（class 2；trace 事件名 `MemoryEvent` 同词）。
#[derive(Envcall)]
#[call(class = 2, fail = MemoryFail)]
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum MemoryCall {
    /// 用户堆分配（字节数，页对齐向上取整）。
    #[ret(VirtAddr)]
    Allocate { size: usize },
    /// 用户堆释放（VA，字节数，页对齐）。
    #[ret(())]
    Deallocate { addr: VirtAddr, size: usize },
    /// 高位大段懒匿名映射（字节数页对齐；at = 期望 VA，VirtAddr(0) = 窗口自选）。
    #[ret(VirtAddr)]
    Mmap { size: usize, at: VirtAddr },
    /// 释放 mmap/声明区域（VA，字节数，页对齐）。
    #[ret(())]
    Munmap { addr: VirtAddr, size: usize },
    /// 修改映射区域保护标志（VA，字节数页对齐，新权限 PteFlags 位）。
    #[ret(())]
    Mprotect {
        addr: VirtAddr,
        size: usize,
        flags: u64,
    },
}
