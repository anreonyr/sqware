/// The bounded startup window before Control consumes the ready signal.
pub struct Startup {
    deadline: u64,
    observed: bool,
}

impl Startup {
    pub fn new(deadline: u64) -> Self {
        Self {
            deadline,
            observed: false,
        }
    }

    pub fn observed(&mut self) {
        self.observed = true;
    }

    /// A denial is fatal once any scope was observed, or after the startup window.
    pub fn deny_is_fatal(&self, now: u64) -> bool {
        self.observed || now >= self.deadline
    }
}
