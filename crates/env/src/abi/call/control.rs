//! call::control — **Control 域（class 6：调度与诊断面）**：调用表（[`ControlCall`]）与失败词汇（[`ControlFail`]）。

use mold::{Envcall, Fail};

/// Control 域（class 6：自诊断）的失败词汇。
#[derive(Fail)]
pub enum ControlFail {
    /// `buf` 非法（未映射 / 不可写）。
    Denied = -1,
}

/// `ControlFail` 的结果别名。
pub type ControlResult<T> = Result<T, erra::Error<ControlFail>>;

/// 控制调用（class 6）。
#[derive(Envcall)]
#[call(class = 6, fail = ControlFail)]
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ControlCall {
    /// 用户自诊断：采样当前任务调用栈，把 pc 地址数组写进用户 buf，返回帧数。
    ///
    /// `buf` = 用户预分配的 `[usize; N]` 数组 VA；`frames` = 该数组最大容量。
    /// 内核经 `mail::copy_out` 写 `frames` 个 pc 到 buf；返回实际捕获帧数（`usize`），
    /// buf 非法（未映射/不可写）→ 负值（`ControlFail`）。
    #[ret(usize)]
    Backtrace { buf: usize, frames: usize },
}
