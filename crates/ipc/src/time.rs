//! Shared monotonic wait-budget accounting.

use env::{Wait, chrono};

/// One monotonic budget shared by every step of an exchange.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Deadline {
    started: u64,
    budget_ms: Option<usize>,
}

impl Deadline {
    pub fn new(wait: Wait) -> Self {
        let budget_ms = match wait {
            Wait::Forever => None,
            Wait::AtMost(ms) => Some(ms),
        };
        Self { started: chrono::clock(), budget_ms }
    }

    /// Remaining time preserves the established floor-to-milliseconds conversion.
    pub fn remaining(self) -> Wait {
        match self.budget_ms {
            None => Wait::Forever,
            Some(ms) => {
                let elapsed = (chrono::clock().saturating_sub(self.started) / 1_000_000) as usize;
                Wait::AtMost(ms.saturating_sub(elapsed))
            }
        }
    }
}

/// Convert a relative wait into an absolute nanosecond deadline for transport waits.
pub fn deadline(wait: Wait) -> u64 {
    match wait {
        Wait::Forever => u64::MAX,
        Wait::AtMost(ms) => chrono::clock().saturating_add((ms as u64).saturating_mul(1_000_000)),
    }
}

/// Compute the remaining wait; `POLL` means the budget is exhausted.
pub fn remain(deadline: u64) -> Wait {
    if deadline == u64::MAX {
        return Wait::Forever;
    }
    let now = chrono::clock();
    if now >= deadline {
        Wait::POLL
    } else {
        Wait::AtMost(((deadline - now) / 1_000_000) as usize)
    }
}
