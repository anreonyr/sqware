//! 一帧推一步的结果。

use super::started::Started;

/// 一帧推一步的结果。
pub(super) enum Stage {
    /// 又推进了一步，接着走。
    Continue,
    /// 这一步要等硬件：本帧到此，下一帧接着来。
    Waiting,
    /// 收工。
    Done(Started),
}
