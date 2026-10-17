//! 在途的那一条 `IOFENCE.C`：本域记下的状态。

use super::handshake::Handshake;

/// 在途的那一条 `IOFENCE.C`：它在完成记录页的那一槽、本域给它的完成值，
/// 以及**那一条**的等待期限。重试只继续等它，不重提新命令、也不重置这条期限。
pub(super) struct Outstanding {
    pub(super) slot: usize,
    pub(super) cookie: u32,
    pub(super) handshake: Handshake,
}
