//! 私有孔句柄：只供正式资源封装组合使用。

use env::{HoleCondition, HoleResult, PieToken, TaskId, Wait};

pub(crate) struct Hole {
    token: PieToken,
}

impl Hole {
    pub(crate) fn from_raw(token: PieToken) -> Self {
        Self { token }
    }

    pub(crate) fn token(&self) -> PieToken {
        self.token
    }

    pub(crate) fn push(&self, bytes: &[u8], within: Wait) -> HoleResult<()> {
        super::raw::Hole::from_raw(self.token).push(bytes, within)
    }

    pub(crate) fn pull(&self, bytes: &mut [u8], within: Wait) -> HoleResult<(usize, TaskId)> {
        super::raw::Hole::from_raw(self.token).pull(bytes, within)
    }

    pub(crate) fn wait(&self, dir: HoleCondition, within: Wait) -> HoleResult<bool> {
        super::raw::Hole::from_raw(self.token).wait(dir, within)
    }

    pub(crate) fn unseal(mark: env::Mark) -> HoleResult<Self> {
        Ok(Self::from_raw(env::hole::unseal(mark, env::HoleLimits::default())?))
    }
}
