use ::core::cell::UnsafeCell;
use ::core::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

use super::depend;

mod guard;

pub use guard::BareLockGuard;

pub struct BareLock<T: ?Sized> {
    locked: AtomicBool,
    caller: AtomicUsize,
    #[cfg_attr(not(debug_assertions), allow(dead_code))]
    level: Option<depend::Level>,
    data: UnsafeCell<T>,
}

unsafe impl<T: ?Sized> Sync for BareLock<T> {}

impl<T> BareLock<T> {
    #[allow(dead_code)]
    pub const fn new(val: T) -> Self {
        BareLock {
            locked: AtomicBool::new(false),
            caller: AtomicUsize::new(0),
            level: None,
            data: UnsafeCell::new(val),
        }
    }

    #[allow(dead_code)]
    pub const fn new_level(level: depend::Level, val: T) -> Self {
        BareLock {
            locked: AtomicBool::new(false),
            caller: AtomicUsize::new(0),
            level: Some(level),
            data: UnsafeCell::new(val),
        }
    }
}

impl<T: ?Sized> BareLock<T> {
    #[allow(dead_code)]
    #[inline(never)]
    pub unsafe fn lock(&self) -> BareLockGuard<'_, T> {
        let caller = crate::machine::sync::depend_enter!(self);
        crate::machine::sync::depend_check!(self, caller);
        while self.locked.swap(true, Ordering::Acquire) {
            crate::machine::stop::hush();
            ::core::hint::spin_loop();
        }
        self.caller.store(caller, Ordering::Relaxed);
        crate::machine::sync::depend_acquire!(self, caller);

        BareLockGuard::new(self)
    }

    #[allow(dead_code)]
    #[inline(never)]
    pub unsafe fn try_lock(&self) -> Option<BareLockGuard<'_, T>> {
        let caller: usize;
        unsafe { ::core::arch::asm!("mv {0}, ra", out(reg) caller) };
        if self.locked.swap(true, Ordering::Acquire) {
            return None;
        }
        self.caller.store(caller, Ordering::Relaxed);
        crate::machine::sync::depend_acquire!(self, caller);

        Some(BareLockGuard::new(self))
    }
}
