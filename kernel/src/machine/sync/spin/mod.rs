use ::core::cell::UnsafeCell;
use ::core::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

use super::depend;
use super::trap::TrapGuard;

mod guard;

pub use guard::SpinLockGuard;

pub struct SpinLock<T: ?Sized> {
    locked: AtomicBool,
    caller: AtomicUsize,
    #[cfg_attr(not(debug_assertions), allow(dead_code))]
    level: Option<depend::Level>,
    data: UnsafeCell<T>,
}

unsafe impl<T: ?Sized> Sync for SpinLock<T> {}

impl<T> SpinLock<T> {
    pub const fn new(val: T) -> Self {
        SpinLock {
            locked: AtomicBool::new(false),
            caller: AtomicUsize::new(0),
            level: None,
            data: UnsafeCell::new(val),
        }
    }

    pub const fn new_level(level: depend::Level, val: T) -> Self {
        SpinLock {
            locked: AtomicBool::new(false),
            caller: AtomicUsize::new(0),
            level: Some(level),
            data: UnsafeCell::new(val),
        }
    }
}

impl<T: ?Sized> SpinLock<T> {
    #[allow(dead_code)]
    pub fn caller(&self) -> usize {
        self.caller.load(Ordering::Relaxed)
    }

    #[inline(never)]
    pub fn lock(&self) -> SpinLockGuard<'_, T> {
        let caller = crate::machine::sync::depend_enter!(self);
        let trap = unsafe { TrapGuard::save() };
        crate::machine::sync::depend_check!(self, caller);

        while self.locked.swap(true, Ordering::Acquire) {
            crate::machine::stop::hush();
            ::core::hint::spin_loop();
        }
        self.caller.store(caller, Ordering::Relaxed);
        crate::machine::sync::depend_acquire!(self, caller);

        SpinLockGuard::new(self, trap)
    }

    /// 共持同级对的第二把：只在已持 `lower` 的前提下取，且按地址序在 `lower` 之后。
    ///
    /// 只给 `sync` 子树里的 `with_pair` 用：它不对外，所以没人能拿了第二把守卫
    /// 又先放低锁，把闭包里那对守卫的生命周期拆开。
    #[inline(never)]
    pub(super) fn lock_ordered<U>(&self, lower: &SpinLock<U>) -> SpinLockGuard<'_, T> {
        let caller = crate::machine::sync::depend_enter!(self);
        let trap = unsafe { TrapGuard::save() };
        crate::machine::sync::depend_check_peer!(self, lower, caller);

        while self.locked.swap(true, Ordering::Acquire) {
            crate::machine::stop::hush();
            ::core::hint::spin_loop();
        }
        self.caller.store(caller, Ordering::Relaxed);
        crate::machine::sync::depend_acquire!(self, caller);

        SpinLockGuard::new(self, trap)
    }

    #[allow(dead_code)]
    #[inline(never)]
    pub fn try_lock(&self) -> Option<SpinLockGuard<'_, T>> {
        let caller: usize;
        unsafe { ::core::arch::asm!("mv {0}, ra", out(reg) caller) };
        let trap = unsafe { TrapGuard::save() };

        if self.locked.swap(true, Ordering::Acquire) {
            return None;
        }
        self.caller.store(caller, Ordering::Relaxed);
        crate::machine::sync::depend_acquire!(self, caller);

        Some(SpinLockGuard::new(self, trap))
    }
}
