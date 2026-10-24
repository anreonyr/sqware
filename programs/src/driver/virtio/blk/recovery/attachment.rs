//! BAR attachment, grant and reset metadata; Executor retains the actual mapping.

use env::{PieToken, TaskId};

use crate::driver::virtio::Caps;

pub struct Attachment {
    pub serial: u64,
    pub execution: TaskId,
    pub pages: u32,
    pub caps: Caps,
    /// reset/drain 做过了吗（重试不重做）。
    pub stopped: bool,
    /// reset 写已发出，下一拍回读状态与 queue-enable。
    pub reset_pending: bool,
    pub bar_grant: Option<PieToken>,
}
