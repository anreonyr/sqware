//! **上限族的期限** —— "时间参数的定式"（[`crate::fid`] 头注那一份）的**类型义务版**。
//!
//! 三态里的"`usize::MAX` = 永久"与"一个很大的毫秒数"在整数里**长得一样**，故它欠一个类型。
//! 两处护栏就是证据（`contract/src/session/core.rs` 的 `(left != usize::MAX).then(…)` 与
//! `.min(usize::MAX as u64)`）：不先判一下，哨兵就会被当成一个真实的毫秒数算进去。
//!
//! **`0` 不是第三格**：它就是 [`Wait::AtMost(0)`]（至多 0 毫秒 = 当场答、不挂起）。
//!
//! **下限族不拿它**：只有 `RoomCall::Park` 谈"至少"，它的参数照旧是裸 `usize`——定式要求
//! "每个时间参数一眼看出属于哪一族"，**两个不同的类型**就是那条要求的落地。
//!
//! 线上那一格的拼法仍只有一处（[`crate::fid`] 头注）；两端各转一次：[`Wait::from_wire`] /
//! [`Wait::to_wire`]（`Wire` 那一对实现在 [`crate::wire`]）。

/// **上限族的期限**：等某事发生，至多等这么久。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Wait {
    /// 至多这么多毫秒；`AtMost(0)` = **只探测**（当场答，不挂起）。
    AtMost(usize),
    /// **永久**：等到条件成立为止。
    Forever,
}

impl Wait {
    /// 至多 0 毫秒：只探测，不挂起。
    pub const POLL: Wait = Wait::AtMost(0);

    /// 线上那一格 → 本类型。**满射**（每个 `usize` 都有一格）⇒ 这一步永不失败。
    pub const fn from_wire(millis: usize) -> Wait {
        if millis == usize::MAX {
            Wait::Forever
        } else {
            Wait::AtMost(millis)
        }
    }

    /// 本类型 → 线上那一格。
    pub const fn to_wire(self) -> usize {
        match self {
            Wait::AtMost(millis) => millis,
            Wait::Forever => usize::MAX,
        }
    }

    /// 内核那一侧：这一格等于等多久。
    pub fn into_duration(self) -> core::time::Duration {
        match self {
            Wait::AtMost(millis) => core::time::Duration::from_millis(millis as u64),
            Wait::Forever => core::time::Duration::MAX,
        }
    }
}
