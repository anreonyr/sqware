//! 本地持有与对端派生授予的独立收尾责任。

use env::{Mark, Permission, PieResult, PieToken, TaskId, pie};

/// 本任务创建的一份能力。释放它不会自动封印资源。
pub struct Capability {
    token: PieToken,
    active: bool,
}

impl Capability {
    pub fn unseal_hole(mark: Mark) -> PieResult<Self> {
        Ok(Self { token: pie::unseal_hole(mark)?, active: true })
    }

    pub fn token(&self) -> PieToken { self.token }

    pub fn grant(&self, peer: TaskId, permission: Permission, mark: Mark) -> PieResult<Loan<'_>> {
        Loan::accord(&self.token, peer, permission, mark)
    }

    pub fn seal(&self) -> PieResult<()> { pie::seal(self.token) }

    pub fn release(mut self) -> PieResult<()> {
        self.active = false;
        pie::release(self.token)
    }
}

impl Drop for Capability {
    fn drop(&mut self) {
        if self.active { let _ = pie::release(self.token); }
    }
}

/// 授给指定对端的一份派生能力。借用源编号，只撤销这次授予。
pub struct Loan<'a> {
    _source: &'a PieToken,
    peer: TaskId,
    remote: PieToken,
    active: bool,
}

impl<'a> Loan<'a> {
    pub fn accord(source: &'a PieToken, peer: TaskId, permission: Permission, mark: Mark) -> PieResult<Self> {
        let remote = pie::accord(*source, peer, permission, mark)?;
        Ok(Self { _source: source, peer, remote, active: true })
    }

    /// 仅供写入对端解释的报文，不是本地可操作句柄。
    pub fn remote(&self) -> PieToken { self.remote }

    pub fn revoke(mut self) -> PieResult<()> {
        self.active = false;
        pie::revoke(self.peer, self.remote)
    }
}

impl Drop for Loan<'_> {
    fn drop(&mut self) {
        if self.active { let _ = pie::revoke(self.peer, self.remote); }
    }
}
