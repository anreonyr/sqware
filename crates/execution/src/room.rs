//! Room operations for parking, waiting, waking and reaping a task.

use core::time::Duration;
use env::{Reason, RoomCall, VirtAddr};

pub use env::room::{wait, wake};

/// Park for at least this duration, rounding fractional milliseconds upward.
pub fn park(duration: Duration) -> env::RoomResult<()> {
    let mut millis = duration.as_millis();
    if duration.subsec_nanos() % 1_000_000 != 0 {
        millis += 1;
    }
    env::room::park(millis.min(usize::MAX as u128) as usize)
}

/// Reap the current task with a reason and an optional diagnostic note.
pub fn reap(reason: Reason, note: Option<&str>) -> ! {
    let note = note.unwrap_or("");
    let _ = RoomCall::Reap {
        reason,
        note: VirtAddr::new(note.as_ptr() as usize),
        len: note.len().min(env::NOTE_MAX),
    }
    .call();
    unreachable!("Reap returned: reason={reason}")
}
