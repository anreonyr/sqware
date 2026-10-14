use ::core::fmt;
use env::PieFail;

pub(crate) struct Failure<T> {
    pub reason: PieFail,
    pub value: T,
}

impl<T> Failure<T> {
    pub(crate) fn into_parts(self) -> (PieFail, T) {
        (self.reason, self.value)
    }
}

impl<T> fmt::Debug for Failure<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.reason.fmt(f)
    }
}
