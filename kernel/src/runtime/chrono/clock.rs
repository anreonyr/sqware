use core::sync::atomic::{AtomicU64, Ordering};
use core::time::Duration;

use fack::prelude::Error;
use riscv::register::time;

use crate::lock::OnceLock;
use crate::platform::machine;

const NANOS_PER_SEC: u128 = 1_000_000_000;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct Instant(u64);

#[derive(Error, Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClockError {
    #[error("no timebase-frequency in device tree")]
    NoTimebase,
    #[error("clock already initialized")]
    AlreadyInit,
}

static HERTZ: OnceLock<u64> = OnceLock::new();
static CYCLE: AtomicU64 = AtomicU64::new(0);

pub fn init() -> Result<(), ClockError> {
    let hertz = machine::info().hart.hertz;
    if hertz == 0 {
        return Err(ClockError::NoTimebase);
    }
    HERTZ
        .set(hertz as u64)
        .map_err(|_| ClockError::AlreadyInit)?;
    CYCLE.store(time::read() as u64, Ordering::Relaxed);
    Ok(())
}

fn hertz() -> u64 {
    HERTZ.get().copied().expect("clock not initialized")
}

pub fn now() -> Instant {
    Instant(time::read() as u64)
}

pub(crate) fn uptime_ticks() -> u64 {
    (time::read() as u64).wrapping_sub(CYCLE.load(Ordering::Relaxed))
}

pub fn uptime() -> Duration {
    let boot = CYCLE.load(Ordering::Relaxed);
    ticks_to_duration((time::read() as u64).wrapping_sub(boot))
}

impl Instant {
    pub fn as_ticks(self) -> u64 {
        self.0
    }

    pub(crate) fn from_ticks(t: u64) -> Instant {
        Instant(t)
    }

    pub fn add(&self, d: Duration) -> Instant {
        Instant(self.0.wrapping_add(duration_to_ticks(d)))
    }
}

pub(crate) fn ticks_to_duration(ticks: u64) -> Duration {
    let ns = (ticks as u128).saturating_mul(NANOS_PER_SEC) / hertz() as u128;
    Duration::from_nanos(ns.min(u64::MAX as u128) as u64)
}

pub(crate) fn duration_to_ticks(d: Duration) -> u64 {
    let ns = d.as_nanos();
    let t = ns.saturating_mul(hertz() as u128) / NANOS_PER_SEC;
    t.min(u64::MAX as u128) as u64
}
