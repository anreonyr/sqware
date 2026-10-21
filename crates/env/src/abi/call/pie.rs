//! call::pie — **Pie 域（class 7，权柄轴：许可的生死与流动）**：调用表（[`PieCall`]）与失败词汇（[`PieFail`]）。

use crate::abi::permission::Permission;
use crate::wire::{Mark, PieToken, TaskId, VirtAddr};
use mold::{Envcall, Fail};

/// Pie 域（class 7：权柄轴）的失败词汇。
#[derive(Fail)]
pub enum PieFail {
    /// 不是开辟者 / 表里没这枚 / 类型不符。
    Denied = -1,
    /// 资源已封印。
    Dead = -2,
    /// 门闩表备不下。
    OoM = -3,
    /// Pole 的 `size` / 区间非法（未页对齐）。
    NotAligned = -4,
    /// 这一枚已交出去（交回即复原）。
    HandedOver = -5,
    #[busy]
    Busy = -6,
}

/// `PieFail` 的结果别名。
pub type PieResult<T> = Result<T, erra::Error<PieFail>>;

pub use super::pie_info::PieInfo;
pub use super::release_mode::ReleaseMode;

/// Capability transfer, queries and release.
#[derive(Envcall)]
#[call(class = 7, fail = PieFail)]
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum PieCall {
    #[slot(4)]
    #[ret(PieToken)]
    Accord {
        src: PieToken,
        dst: TaskId,
        subset: Permission,
        mark: Mark,
    },
    #[slot(5)]
    #[ret(())]
    Narrow { token: PieToken, subset: Permission },
    #[slot(6)]
    #[ret(())]
    Revoke { dst: TaskId, token: PieToken },
    /// Keep removes a borrowed, non-exclusive reference and reparents its children.
    /// Revoke removes the reference and all descendants. Both work after Seal.
    #[slot(7)]
    #[ret(())]
    Release { token: PieToken, mode: ReleaseMode },
    /// Write seven words (PieInfo) to buf. Closed resources can still be inspected.
    #[slot(8)]
    #[ret(())]
    Inspect { token: PieToken, buf: VirtAddr },
    /// Write at most capacity records in increasing token order, strictly after after.
    /// Returns the number written. Each batch observes current state, not a frozen table.
    #[slot(9)]
    #[ret(usize)]
    Collect {
        after: PieToken,
        buf: VirtAddr,
        capacity: usize,
    },
    #[slot(10)]
    #[ret(bool)]
    Same { a: PieToken, b: PieToken },
    /// 同 `Accord`，但给这一枚子分支一个**有效窗口**：`offset` 是相对来源起点的
    /// 绝对页偏移，`count` 是页数；必须落在父的有效窗口里（只收窄）。
    #[slot(11)]
    #[ret(PieToken)]
    AccordWindow {
        src: PieToken,
        dst: TaskId,
        subset: Permission,
        mark: Mark,
        offset: usize,
        count: usize,
    },
}
