//! call::memory — **Memory 域（class 2：地址空间与页）**：调用表（[`MemoryCall`]）与失败词汇（[`MemoryFail`]）。

use crate::wire::{PieToken, TeamId, VirtAddr};
use mold::{Envcall, Fail};

/// 环境内存调用与装载格式使用的页粒度。
pub const PAGE_SIZE: usize = 4096;

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
    #[busy]
    Busy = -7,
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
    /// 安装页映射；team=0 是当前域，非零必须是调用者的 Constructing 子域。
    /// at=0 自动选址；size/offset 按页对齐；flags 是 R/W/X 位 1/2/3。
    /// backing=NONE 创建 R/RW lazy-zero；Pole 提供受授权上限约束的物理页。
    /// 共享程序页无写授权；ONLY 根页在首次 Spawn 时原子消费。
    #[ret(VirtAddr)]
    Mmap {
        team: TeamId,
        at: VirtAddr,
        size: usize,
        backing: PieToken,
        offset: usize,
        flags: u64,
    },
    /// 释放 mmap/声明区域（VA，字节数，页对齐）。
    #[ret(())]
    Munmap {
        team: TeamId,
        addr: VirtAddr,
        size: usize,
    },
    /// 修改映射区域保护标志（VA，字节数页对齐，新权限仅 R/W/X：位 1/2/3）。
    /// 内核管理 V/U/G/A/D；拒绝空权限及没有 R 的 W。
    #[ret(())]
    Mprotect {
        team: TeamId,
        addr: VirtAddr,
        size: usize,
        flags: u64,
    },
}
