//! Tole resource wrapper. Registration and multi-source waiting belong to Mail.
use env::{AwaitReply, MailResult, PieToken, Source, Wait};
pub struct Pile {
    pie: PieToken,
}
impl Pile {
    pub fn unseal(shared: bool) -> env::PieResult<Self> {
        Ok(Self {
            pie: env::pie::unseal(env::UnsealArgs::Tole { shared })?,
        })
    }
    pub fn from_raw(pie: PieToken) -> Self {
        Self { pie }
    }
    pub fn attach(&self, source: Source) -> MailResult<()> {
        env::mail::attach(self.pie, source)
    }
    pub fn detach(&self, source: Source) -> MailResult<()> {
        env::mail::detach(self.pie, source)
    }
    pub fn await_(&self, wait: Wait) -> MailResult<AwaitReply> {
        env::mail::await_(self.pie, wait)
    }
    pub fn token(&self) -> PieToken {
        self.pie
    }
}
