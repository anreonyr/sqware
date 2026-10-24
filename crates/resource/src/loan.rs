//! 授给指定对端的一份派生能力。借用源编号，只撤销这次授予。

use env::{Mark, Permission, PieResult, PieToken, TaskId, pie};

pub struct Loan<'a> {
    _source: &'a PieToken,
    peer: TaskId,
    remote: PieToken,
    active: bool,
}

impl<'a> Loan<'a> {
    pub fn accord(
        source: &'a PieToken,
        peer: TaskId,
        permission: Permission,
        mark: Mark,
    ) -> PieResult<Self> {
        let remote = pie::accord(*source, peer, permission, mark)?;
        Ok(Self {
            _source: source,
            peer,
            remote,
            active: true,
        })
    }

    /// 仅供写入对端解释的报文，不是本地可操作句柄。
    pub fn remote(&self) -> PieToken {
        self.remote
    }

    /// Leave the delivered capability with its recipient.
    pub fn keep(mut self) -> PieToken {
        self.active = false;
        self.remote
    }

    /// 失败时保留同一对端与 token，调用方可用同一句柄重试。
    pub fn revoke(&mut self) -> PieResult<()> {
        if !self.active {
            return Ok(());
        }
        crate::raw::revoke(self.peer, self.remote)?;
        self.active = false;
        Ok(())
    }
}

impl Drop for Loan<'_> {
    fn drop(&mut self) {
        if self.active {
            let _ = crate::raw::revoke(self.peer, self.remote);
        }
    }
}
