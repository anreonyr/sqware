//! 一次交互使用的自有回信端。

use env::{MailFail, MailResult, Mark, Permission, PieResult, TaskId, Wait, make_fail};
use crate::{hole::Hole, scope::{Grant, Owned}};

/// 自有回信端；只接收指定对端的回复，结束时封印并释放。
pub struct Reply {
    local: Owned,
    peer: TaskId,
    mark: Mark,
}

impl Reply {
    pub fn open(peer: TaskId, mark: Mark) -> PieResult<Self> {
        Ok(Self { local: Owned::hole(mark)?, peer, mark })
    }

    pub fn grant(&self) -> PieResult<Grant<'_>> {
        self.local.grant(self.peer, Permission::STORE, self.mark)
    }

    pub fn pull<'a>(&self, buffer: &'a mut [u8], within: Wait) -> MailResult<&'a [u8]> {
        let (len, from) = Hole::from_token(self.local.token()).pull(buffer, within)?;
        if from != self.peer { return Err(make_fail(MailFail::Denied)); }
        buffer.get(..len).ok_or_else(|| make_fail(MailFail::Denied))
    }
}

impl Drop for Reply {
    fn drop(&mut self) { let _ = self.local.seal(); }
}
