//! 本任务持有的一份能力及其本地派生。

use env::{FlatReference, GrantRange, Mark, Permission, PieResult, PieToken, TaskId, pie};

use crate::loan::Loan;

/// 本任务创建的一份能力。释放它不会自动封印资源。
pub struct Capability {
    token: PieToken,
    active: bool,
}

impl Capability {
    pub fn unseal_hole(mark: Mark) -> env::HoleResult<Self> {
        Ok(Self {
            token: env::hole::unseal(mark, env::HoleLimits::default())?,
            active: true,
        })
    }

    /// Turn one exact source window into its own independent root.
    ///
    /// The kernel rejects overlapping child windows and keeps the parent's
    /// exclusive delivery unused; the returned root is what may be granted.
    pub fn unseal_pole(reference: FlatReference, shared: bool) -> env::PoleResult<Self> {
        Ok(Self {
            token: env::pole::unseal(reference, shared)?,
            active: true,
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
    ) -> PieResult<Self> {
        Ok(Self {
            token: pie::derive(source, permission, mark, range)?,
            active: true,
        })
    }

    pub fn grant(&self, peer: TaskId, permission: Permission, mark: Mark) -> PieResult<Loan<'_>> {
        Loan::accord(&self.token, peer, permission, mark)
    }

    pub fn seal(&self) -> env::HoleResult<()> {
        env::hole::seal(self.token)
    }

    /// 失败时保留同一枚 token，调用方可用同一句柄重试。
    pub fn release(&mut self) -> PieResult<()> {
        if !self.active {
            return Ok(());
        }
        crate::raw::release(self.token)?;
        self.active = false;
        Ok(())
    }
}

impl Drop for Capability {
    fn drop(&mut self) {
        if self.active {
            let _ = crate::raw::release(self.token);
        }
    }
}
