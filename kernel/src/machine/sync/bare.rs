use ::core::cell::UnsafeCell;
use ::core::marker::PhantomData;
use ::core::ops::{Deref, DerefMut};
use ::core::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

use super::depend;

pub struct BareLock<T: ?Sized> {
    locked: AtomicBool,
    caller: AtomicUsize,
    #[cfg_attr(not(debug_assertions), allow(dead_code))]
    level: Option<depend::Level>,
    data: UnsafeCell<T>,
}

unsafe impl<T: ?Sized> Sync for BareLock<T> {}

pub struct BareLockGuard<'a, T: ?Sized> {
    lock: &'a BareLock<T>,
    _not_send: PhantomData<*const ()>,
}

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

        BareLockGuard {
            lock: self,
            _not_send: PhantomData,
        }
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

        Some(BareLockGuard {
            lock: self,
            _not_send: PhantomData,
        })
    }
}

impl<T: ?Sized> Deref for BareLockGuard<'_, T> {
    type Target = T;

    fn deref(&self) -> &T {
        unsafe { &*self.lock.data.get() }
    }
}

impl<T: ?Sized> DerefMut for BareLockGuard<'_, T> {
    fn deref_mut(&mut self) -> &mut T {
        unsafe { &mut *self.lock.data.get() }
    }
}

impl<T: ?Sized> Drop for BareLockGuard<'_, T> {
    fn drop(&mut self) {
        crate::machine::sync::depend_release!(self.lock);
        self.lock.locked.store(false, Ordering::Release);
        self.lock.caller.store(0, Ordering::Relaxed);
    }
}
