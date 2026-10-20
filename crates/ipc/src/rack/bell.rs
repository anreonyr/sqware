//! One independent notification on a Pole. Ring/Hush are idempotent;
//! Wait observes without clearing. Data readiness remains in the shared ring.
use env::{Bit, PoleCondition, PoleResult, PieToken, Wait};
pub(crate) struct Bell {
    pie: PieToken,
    bit: Bit,
}
impl Bell {
    pub(crate) fn from_raw(pie: PieToken, bit: Bit) -> Self {
        Self { pie, bit }
    }
    pub(crate) fn token(&self) -> PieToken {
        self.pie
    }
    pub(crate) fn source(&self) -> env::ToleEventSource {
        env::ToleEventSource::Pole {
            token: self.pie,
            bit: self.bit,
        }
    }
    pub(crate) fn ring(&self) -> PoleResult<()> {
        env::pole::ring(self.pie, self.bit)
    }
    pub(crate) fn hush(&self) -> PoleResult<()> {
        env::pole::hush(self.pie, self.bit)
    }
    pub(crate) fn wait(&self, wait: Wait) -> PoleResult<bool> {
        env::pole::wait(self.pie, PoleCondition::Signal(self.bit), wait)
    }
}
