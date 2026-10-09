//! 一次交互使用的自有回信端。

use env::{MailFail, Mark, Permission, PieFail, PieResult, PieToken, TaskId, Wait, make_fail, pie};
use crate::{hole::Hole, capability::Capability};

/// 自有回信端；只接收指定对端的回复，结束时撤销授出并封印释放本地孔。
pub struct Reply {
    local: Capability,
    peer: TaskId,
    mark: Mark,
    remote: Option<PieToken>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ReplyError {
    Mail(MailFail),
    WrongSource,
}

impl Reply {
    pub fn open(peer: TaskId, mark: Mark) -> PieResult<Self> {
        Ok(Self { local: Capability::unseal_hole(mark)?, peer, mark, remote: None })
    }

    /// Create one grant owned by this reply endpoint and return its remote token.
    pub fn grant(&mut self) -> PieResult<PieToken> {
        if self.remote.is_some() {
            return Err(make_fail(PieFail::Denied));
        }
        let remote = pie::accord(self.local.token(), self.peer, Permission::STORE, self.mark)?;
        self.remote = Some(remote);
        Ok(remote)
    }

    pub fn pull<'a>(&self, buffer: &'a mut [u8], within: Wait) -> Result<&'a [u8], ReplyError> {
        let (len, from) = Hole::from_raw(self.local.token()).pull(buffer, within)
            .map_err(|error| ReplyError::Mail(error.source))?;
        if from != self.peer { return Err(ReplyError::WrongSource); }
        buffer.get(..len).ok_or(ReplyError::Mail(MailFail::Denied))
    }
}

impl Drop for Reply {
    fn drop(&mut self) {
        if let Some(remote) = self.remote.take() {
            let _ = crate::raw::revoke(self.peer, remote);
        }
        let _ = self.local.seal();
    }
}
