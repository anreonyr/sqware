use crate::Wait;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Deadline(ipc_impl::time::Deadline);

impl Deadline {
    pub fn new(wait: Wait) -> Self {
        Self(ipc_impl::time::Deadline::new_with_clock(
            wait,
            crate::chrono::clock,
        ))
    }

    pub fn remaining(self) -> Wait {
        self.0.remaining_with_clock(crate::chrono::clock)
    }
}
