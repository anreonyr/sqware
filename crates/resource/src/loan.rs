//! 授给指定对端的一份派生能力。借用源编号，只撤销这次授予。

use core::marker::PhantomData;

use env::{Mark, Permission, PieToken, TaskId};

use crate::{cleanup::cleanup, kernel::CapabilityKernel};

pub struct Loan<'a, K: CapabilityKernel> {
    _source: &'a PieToken,
    peer: TaskId,
    remote: PieToken,
    active: bool,
    _kernel: PhantomData<fn() -> K>,
}

impl<'a, K: CapabilityKernel> Loan<'a, K> {
    pub fn accord(
        source: &'a PieToken,
        peer: TaskId,
        permission: Permission,
        mark: Mark,
    ) -> Result<Self, K::PieError> {
        let remote = K::accord(*source, peer, permission, mark)?;
        Ok(Self {
            _source: source,
            peer,
            remote,
            active: true,
            _kernel: PhantomData,
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
    pub fn revoke(&mut self) -> Result<(), K::PieError> {
        if !self.active {
            return Ok(());
        }
        cleanup::<K>(|| K::revoke(self.peer, self.remote))?;
        self.active = false;
        Ok(())
    }
}

impl<K: CapabilityKernel> Drop for Loan<'_, K> {
    fn drop(&mut self) {
        if self.active {
            let _ = cleanup::<K>(|| K::revoke(self.peer, self.remote));
        }
    }
}
