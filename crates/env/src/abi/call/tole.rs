//! call::tole — **Tole 域（class 9：多路等待）**：调用表（[`ToleCall`]）与失败词汇（[`ToleFail`]）。

use super::MailCondition;
use crate::abi::wait::Wait;
use crate::wire::{PieToken, TaskId};
use mold::{Envcall, Fail};

/// Tole 域（class 9：多路等待）的失败词汇。
#[derive(Fail)]
pub enum ToleFail {
    /// token 不在表里 / 不是组 / 权不够。
    Denied = -1,
    /// 组已封印。
    Dead = -2,
    /// 组表备不下。
    OoM = -3,
    /// 这一枚已交出去（交回即复原）。
    HandedOver = -4,
    /// 条件未就绪（`Await` 的等待位在退化上下文里答这一枚）。
    #[busy]
    Busy = -5,
}

/// `ToleFail` 的结果别名。
pub type ToleResult<T> = Result<T, erra::Error<ToleFail>>;

/// **状态订阅观察的来源**（线上那一格：`0` = 任务收尾、`1` = 能力变化）。
///
/// 判别号只写一处（[`Source::wire`] / [`Source::of`] 在类型自己身上，`crate::wire` 那一对
/// impl 只转调）——与 `PieKind::of` 同一条理由：两处各写一遍 `match 0/1` 就是两份判别号表，
/// 日后加一格必漏一处。
///
/// **两格的 `target` 不同类**：`TaskCompleted` 的那一格是**被观察的任务**；
/// `CapabilitiesChanged` 的那一格必须等于**调用者自己**——内核核，不认别人。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Source {
    /// 观察指定 Task 的**退出收尾完成**。
    ///
    /// 通知时刻**不早于**退出钩子跑完且 `Reaped` 已发布；它不宣称栈、帧、Team、Space
    /// 已经归还，也不改动 conductor 的恰好一次计数。观察范围**不比 `Join` 宽**
    /// （授权判据逐条复用 `Join` 那一条）。
    TaskCompleted,
    /// 观察**调用者自己能力的可观察状态改变**。
    ///
    /// 覆盖会影响下一次 `Inspect`、LINK 选择或权限表枚举结果的**成功**变更：
    /// 外来 Accord 到达、`Release`/`Revoke` 及级联摘除、`Narrow`、`Seal`、
    /// 本地创建与移交。失败与纯查询（`Collect`/`Inspect`）不产生通知。
    /// 通知只要求**复核**，不代替任何判据。
    CapabilitiesChanged,
}

impl Source {
    /// 本类型 → 线上那一格。
    pub const fn wire(self) -> usize {
        match self {
            Source::TaskCompleted => 0,
            Source::CapabilitiesChanged => 1,
        }
    }

    /// 线上那一格 → 本类型（**判别号不认识 ⇒ `None`**：读的人按"这一帧读不懂"处置，不猜）。
    pub const fn of(raw: usize) -> Option<Source> {
        match raw {
            0 => Some(Source::TaskCompleted),
            1 => Some(Source::CapabilitiesChanged),
            _ => None,
        }
    }
}

/// 多路等待：挂入、摘除、等待成员条件，订阅与取消状态来源。
/// 组的创建和寿命由 Pie::Unseal / Seal / Release 管理。
/// Hole 支持 Pull、Push、Empty；Nole 和 Pole 支持 Pull；组不能嵌套。
/// ONLY 组移交后源引用不可操作；共享组的唤醒只是复核提示。
#[derive(Envcall)]
#[call(class = 9, fail = ToleFail)]
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ToleCall {
    /// Group STORE and member permission for the selected condition are required.
    #[slot(0)]
    #[ret(())]
    Attach {
        tole: PieToken,
        pie: PieToken,
        condition: MailCondition,
    },
    #[slot(1)]
    #[ret(())]
    Detach {
        tole: PieToken,
        pie: PieToken,
        condition: MailCondition,
    },
    /// NONE is a recheck hint after parking or a state notification, not proof of timeout.
    #[slot(2)]
    #[ret((PieToken, MailCondition))]
    Await { tole: PieToken, millis: Wait },
    #[slot(3)]
    #[ret(())]
    Subscribe {
        tole: PieToken,
        source: Source,
        target: TaskId,
    },
    #[slot(4)]
    #[ret(())]
    Unsubscribe {
        tole: PieToken,
        source: Source,
        target: TaskId,
    },
}
