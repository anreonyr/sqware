use ::core::time::Duration;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct Instant(u64);

impl Instant {
    pub fn as_ticks(self) -> u64 {
        self.0
    }

    pub(crate) fn from_ticks(t: u64) -> Instant {
        Instant(t)
    }

    pub fn add(&self, d: Duration) -> Instant {
        Instant(self.0.wrapping_add(super::duration_to_ticks(d)))
    }
}
