//! call::memory — **Memory 域（class 2：地址空间与页）**：调用表（[`MemoryCall`]）与失败词汇（[`MemoryFail`]），
//! 以及新生命周期调用的载荷类型（一型一文件）。

mod acquire_source;
mod allocation;
mod error;
mod placement;
mod placement_at;
mod placement_error;
mod reference_range;
mod view_control;
mod view_selection;

pub use acquire_source::AcquireSource;
pub use allocation::Allocation;
pub use error::{MemoryFail, MemoryResult};
pub use placement::Placement;
pub use placement_at::PlacementAt;
pub use placement_error::PlacementError;
pub use reference_range::ReferenceRange;
pub use view_control::ViewControl;
pub use view_selection::ViewSelection;

use crate::wire::Field;
use crate::wire::{Decode, FlatReference, PieToken, TeamId, ViewId, VirtAddr};
use mold::Envcall;

/// 环境内存调用与装载格式使用的页粒度。
pub const PAGE_SIZE: usize = 4096;

pub(super) fn store_words<const N: usize>(words: [usize; N], out: &mut [u8]) {
    assert_eq!(out.len(), N * u64::WIDTH);
    for (word, bytes) in words.into_iter().zip(out.chunks_exact_mut(u64::WIDTH)) {
        (word as u64).store(bytes);
    }
}

pub(super) fn fetch_words<const N: usize>(bytes: &[u8]) -> Option<[usize; N]> {
    if bytes.len() != N * u64::WIDTH {
        return None;
    }
    let mut words = [0; N];
    for (word, bytes) in words.iter_mut().zip(bytes.chunks_exact(u64::WIDTH)) {
        *word = usize::try_from(u64::fetch(bytes)?).ok()?;
    }
    Some(words)
}

/// 范围坐标的唯一 checked 口径：`count` 非零，且偏移、页数与字节跨度都不溢出。
///
/// 页窗口的两处用家（[`ReferenceRange`] 与 [`ViewSelection`]）共用它，
/// 线外不接受靠溢出缩短范围的输入。
pub(super) fn pages(offset: usize, count: usize) -> Result<(), Decode> {
    if count == 0 {
        return Err(Decode::Invalid);
    }
    count.checked_mul(PAGE_SIZE).ok_or(Decode::Invalid)?;
    let end = offset.checked_add(count).ok_or(Decode::Invalid)?;
    end.checked_mul(PAGE_SIZE).ok_or(Decode::Invalid)?;
    Ok(())
}

/// 内存调用（class 2；trace 事件名 `MemoryEvent` 同词）。
///
/// 操作号为 5–9、11；10 留给 Transfer，0–4 已退役。
#[derive(Envcall)]
#[call(class = 2, fail = MemoryFail)]
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum MemoryCall {
    /// 登记一份经过授权的来源持有；返回它在本任务资源表里的句柄。
    /// `retention=NONE` 是普通取得；否则它是一枚内核认可的独立保留授权。
    #[slot(5)]
    #[ret(FlatReference)]
    Acquire {
        source: AcquireSource,
        retention: PieToken,
    },
    /// 解除这一份持有；不隐式 Detach，不等待其他引用，不收回其他 View。
    #[slot(6)]
    #[ret(())]
    Release { reference: FlatReference },
    /// 在目标地址空间里预留地址并建立翻译关系；返回 (关系句柄, 视图起点)。
    /// 内部组合 Reserve → Attach，不增加独立 Reserve 调用号。
    /// `range` 与 `placement` 分别是指向定长描述的只读用户指针，从**调用者**的
    /// 地址空间各读一次快照；`control` 是这一段映射的控制权选择。
    #[slot(7)]
    #[ret((ViewId, VirtAddr))]
    Attach {
        range: VirtAddr,
        team: TeamId,
        placement: VirtAddr,
        access: u64,
        control: ViewControl,
    },
    /// 按身份安全撤销选定的映射区间。
    #[slot(8)]
    #[ret(())]
    Detach {
        team: TeamId,
        view: ViewId,
        selection: ViewSelection,
    },
    /// 修改选定区间的逻辑权限。
    #[slot(9)]
    #[ret(())]
    SetAccess {
        team: TeamId,
        view: ViewId,
        selection: ViewSelection,
        access: u64,
    },
    /// 遍历本 Task 的驻留引用窗口，输出每页一个小端 u64 PA。
    /// `range` 指向定长 ReferenceRange；`out` 须提供 count * 8 个可写字节。
    /// 当场验证活保留授权；Fresh 可物化，Missing 不得清零恢复。
    /// PA 只在驻留持有有效期间使用，不构成设备访问授权。
    #[slot(11)]
    #[ret(())]
    Traverse {
        retention: PieToken,
        range: VirtAddr,
        out: VirtAddr,
    },
}
