//! 一次交互使用的自有回信端。

use env::{MailFail, Mark, Permission, PieResult, TaskId, Wait};
use crate::{hole::Hole, capability::{Loan, Capability}};

/// 自有回信端；只接收指定对端的回复，结束时封印并释放。
pub struct Reply {
    local: Capability,
    peer: TaskId,
    mark: Mark,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ReplyError {
    Mail(MailFail),
    WrongSource,
}

impl Reply {
    pub fn open(peer: TaskId, mark: Mark) -> PieResult<Self> {
        Ok(Self { local: Capability::unseal_hole(mark)?, peer, mark })
    }

    pub fn grant(&self) -> PieResult<Loan<'_>> {
        self.local.grant(self.peer, Permission::STORE, self.mark)
    }

    pub fn pull<'a>(&self, buffer: &'a mut [u8], within: Wait) -> Result<&'a [u8], ReplyError> {
        let (len, from) = Hole::from_raw(self.local.token()).pull(buffer, within)
            .map_err(|error| ReplyError::Mail(error.source))?;
        if from != self.peer { return Err(ReplyError::WrongSource); }
        buffer.get(..len).ok_or(ReplyError::Mail(MailFail::Denied))
    }
}

impl Drop for Reply {
    fn drop(&mut self) { let _ = self.local.seal(); }
}
