use crate::machine::hart::HartId;

/// 一次状态迁移做完之后留给调用方的事：状态机自己只改轴，轴外的效果（发射、停
/// 泊、叫醒核）由 `Task` 按这里的选择执行。
#[derive(Clone, Copy, Debug)]
pub(crate) enum Step {
    /// 状态机已经摆好（收下 / 恢复就位），没有轴外的事。
    Done,
    /// 轴已按 Held 摆好：调用方走释放路径。
    Held,
    /// 轴已按 Ready 摆好：调用方走发射路径。
    Ready,
    /// 轴是 Ready 且在饥饿链上：调用方先摘链再停泊。
    LeaveReady,
    /// 轴在核上：调用方叫醒那颗核。
    Nudge(HartId),
}
