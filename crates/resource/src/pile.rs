//! Tole registration and multi-source waiting.
use env::{PieToken, ToleAwaitReply, ToleEventSource, ToleResult, Wait};
pub struct Pile {
    pie: PieToken,
}
impl Pile {
    pub fn unseal(shared: bool) -> ToleResult<Self> {
        Ok(Self {
            pie: env::tole::unseal(shared)?,
        })
    }
    pub fn from_raw(pie: PieToken) -> Self {
        Self { pie }
    }
    pub fn attach(&self, source: ToleEventSource) -> ToleResult<()> {
        env::tole::attach(self.pie, source)
    }
    pub fn detach(&self, source: ToleEventSource) -> ToleResult<()> {
        env::tole::detach(self.pie, source)
    }
    pub fn await_(&self, wait: Wait) -> ToleResult<ToleAwaitReply> {
        env::tole::await_(self.pie, wait)
    }
    pub fn token(&self) -> PieToken {
        self.pie
    }
}
