use fack::prelude::Error;

#[derive(Error, Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClockError {
    #[error("no timebase-frequency in device tree")]
    NoTimebase,
    #[error("clock already initialized")]
    AlreadyInit,
}
