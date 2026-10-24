use core::time::Duration;

const MIN_MS: usize = 4;
const MAX_MS: usize = 100;

/// Bounded exponential delay between failed RPC attempts.
pub struct Backoff {
    millis: usize,
}

impl Backoff {
    pub fn new() -> Self {
        Self { millis: MIN_MS }
    }

    pub fn wait(&mut self) {
        let _ = execution::room::park(Duration::from_millis(self.millis as u64));
        self.millis = (self.millis * 2).min(MAX_MS);
    }

    pub fn reset(&mut self) {
        self.millis = MIN_MS;
    }
}
