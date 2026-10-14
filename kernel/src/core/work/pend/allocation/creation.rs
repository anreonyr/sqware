use core::sync::atomic::{AtomicU8, Ordering};

const AVAILABLE: u8 = 0;
const CLAIMED: u8 = 1;
const CONSUMED: u8 = 2;

pub(super) struct Creation {
    state: AtomicU8,
}

impl Creation {
    pub(super) fn new() -> Self {
        Self {
            state: AtomicU8::new(AVAILABLE),
        }
    }

    pub(super) fn claim(&self) -> Result<(), env::PieFail> {
        match self.state.compare_exchange(
            AVAILABLE, CLAIMED, Ordering::AcqRel, Ordering::Acquire,
        ) {
            Ok(_) => Ok(()),
            Err(CLAIMED) => Err(env::PieFail::Busy),
            Err(_) => Err(env::PieFail::Denied),
        }
    }

    pub(super) fn consume(&self) {
        self.state.store(CONSUMED, Ordering::Release);
    }

    pub(super) fn release(&self) {
        let _ = self.state.compare_exchange(
            CLAIMED, AVAILABLE, Ordering::AcqRel, Ordering::Acquire,
        );
    }
}
