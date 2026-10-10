//! One independent notification on a Pole. Ring/Hush are idempotent;
//! Wait observes without clearing. Data readiness remains in the shared ring.
use env::{Bit, MailResult, PieToken, Wait};
pub(crate) struct Bell { pie: PieToken, bit: Bit }
impl Bell {
    pub(crate) fn from_raw(pie: PieToken, bit: Bit) -> Self { Self { pie, bit } }
    pub(crate) fn token(&self) -> PieToken { self.pie }
    pub(crate) fn source(&self) -> env::Source { env::Source::Mail { pie: self.pie, condition: env::MailCondition::Signal(self.bit) } }
    pub(crate) fn ring(&self) -> MailResult<()> { env::mail::ring(self.pie, self.bit) }
    pub(crate) fn hush(&self) -> MailResult<()> { env::mail::hush(self.pie, self.bit) }
    pub(crate) fn wait(&self, wait: Wait) -> MailResult<bool> { env::mail::wait(self.pie, env::MailCondition::Signal(self.bit), wait) }
}
