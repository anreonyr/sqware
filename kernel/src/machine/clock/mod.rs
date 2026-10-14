pub(crate) mod timer;
mod error;
mod instant;

pub use error::ClockError;
pub use instant::Instant;

use ::core::sync::atomic::{AtomicU64, Ordering};
use ::core::time::Duration;

use riscv::register::time;

use crate::machine::board;
use crate::machine::sync::OnceLock;

const NANOS_PER_SEC: u128 = 1_000_000_000;

static HERTZ: OnceLock<u64> = OnceLock::new();
static CYCLE: AtomicU64 = AtomicU64::new(0);

pub fn init() -> Result<(), ClockError> {
    let hertz = board::info().hart.hertz;
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
    Instant::from_ticks(time::read() as u64)
}

pub(crate) fn uptime_ticks() -> u64 {
    (time::read() as u64).wrapping_sub(CYCLE.load(Ordering::Relaxed))
}

pub fn uptime() -> Duration {
    let boot = CYCLE.load(Ordering::Relaxed);
    ticks_to_duration((time::read() as u64).wrapping_sub(boot))
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
