//! 本任务持有的一份能力及其本地派生。

use core::marker::PhantomData;

use env::{FlatReference, GrantRange, Mark, Permission, PieToken, TaskId};

use crate::{cleanup::cleanup, kernel::CapabilityKernel, loan::Loan};

/// 本任务创建的一份能力。释放它不会自动封印资源。
pub struct Capability<K: CapabilityKernel> {
    token: PieToken,
    active: bool,
    _kernel: PhantomData<fn() -> K>,
}

impl<K: CapabilityKernel> Capability<K> {
    pub fn unseal_hole(mark: Mark) -> Result<Self, K::HoleError> {
        Ok(Self {
            token: K::unseal_hole(mark)?,
            active: true,
            _kernel: PhantomData,
        })
    }

    /// Turn one exact source window into its own independent root.
    ///
    /// The kernel rejects overlapping child windows and keeps the parent's
    /// exclusive delivery unused; the returned root is what may be granted.
    pub fn unseal_pole(reference: FlatReference, shared: bool) -> Result<Self, K::PoleError> {
        Ok(Self {
            token: K::unseal_pole(reference, shared)?,
            active: true,
            _kernel: PhantomData,
        })
    }

    pub fn token(&self) -> PieToken {
        self.token
    }

    /// 从本任务的来源 token 按显式范围派生一份拥有的子能力。
    pub fn derive(
        source: PieToken,
        permission: Permission,
        mark: Mark,
        range: GrantRange,
    ) -> Result<Self, K::PieError> {
        Ok(Self {
            token: K::derive(source, permission, mark, range)?,
            active: true,
            _kernel: PhantomData,
        })
    }

    pub fn grant(
        &self,
        peer: TaskId,
        permission: Permission,
        mark: Mark,
    ) -> Result<Loan<'_, K>, K::PieError> {
        Loan::accord(&self.token, peer, permission, mark)
    }

    pub fn seal(&self) -> Result<(), K::HoleError> {
        K::seal_hole(self.token)
    }

    /// 失败时保留同一枚 token，调用方可用同一句柄重试。
    pub fn release(&mut self) -> Result<(), K::PieError> {
        if !self.active {
            return Ok(());
        }
        cleanup::<K>(|| K::release(self.token))?;
        self.active = false;
        Ok(())
    }
}

impl<K: CapabilityKernel> Drop for Capability<K> {
    fn drop(&mut self) {
        if self.active {
            let _ = cleanup::<K>(|| K::release(self.token));
        }
    }
}
