use core::sync::atomic::AtomicBool;

pub(super) struct StopState {
    pub armed: AtomicBool,
    pub kill: AtomicBool,
    pub ruin: AtomicBool,
    pub dead: AtomicBool,
    pub failed: AtomicBool,
}

impl StopState {
    pub(super) fn new() -> Self {
        Self {
            armed: AtomicBool::new(false),
            kill: AtomicBool::new(false),
            ruin: AtomicBool::new(false),
            dead: AtomicBool::new(false),
            failed: AtomicBool::new(false),
        }
    }
}
