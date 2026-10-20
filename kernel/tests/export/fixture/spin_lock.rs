use super::clock::TICKS;
use std::sync::{Mutex, MutexGuard};
use std::sync::atomic::{AtomicUsize, Ordering};

pub static BUSY: AtomicUsize = AtomicUsize::new(0);

pub struct SpinLock<T>(Mutex<T>);

impl<T> SpinLock<T> {
    pub const fn new(value: T) -> Self {
        Self(Mutex::new(value))
    }

    pub fn try_lock(&self) -> Option<MutexGuard<'_, T>> {
        if BUSY.try_update(Ordering::Relaxed, Ordering::Relaxed, |left| left.checked_sub(1)).is_ok() {
            TICKS.fetch_add(10_000, Ordering::Relaxed);
            return None;
        }
        self.0.try_lock().ok()
    }
}
