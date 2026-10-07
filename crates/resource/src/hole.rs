//! 私有孔句柄：只供正式资源封装组合使用。

use env::{HoleDir, MailResult, PieResult, PieToken, Wait, TaskId};

pub(crate) struct Hole {
    token: PieToken,
}

impl Hole {
    pub(crate) fn from_token(token: PieToken) -> Self {
        Self { token }
    }

    pub(crate) fn token(&self) -> PieToken {
        self.token
    }

    pub(crate) fn push(&self, bytes: &[u8], within: Wait) -> MailResult<()> {
        super::raw::HolePie::from_token(self.token).push(bytes, within)
    }

    pub(crate) fn pull(&self, bytes: &mut [u8], within: Wait) -> MailResult<(usize, TaskId)> {
        super::raw::HolePie::from_token(self.token).pull(bytes, within)
    }

    pub(crate) fn wait(&self, dir: HoleDir, within: Wait) -> MailResult<bool> {
        super::raw::HolePie::from_token(self.token).wait(dir, within)
    }

    pub(crate) fn unseal(mark: env::Mark) -> PieResult<Self> {
        Ok(Self::from_token(env::pie::unseal_hole(mark)?))
    }
}
