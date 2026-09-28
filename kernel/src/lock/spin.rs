use core::cell::UnsafeCell;
use core::marker::PhantomData;
use core::ops::{Deref, DerefMut};
use core::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

use super::depend;
use super::trap::TrapGuard;

pub struct SpinLock<T: ?Sized> {
    locked: AtomicBool,
    caller: AtomicUsize,
    #[cfg_attr(not(debug_assertions), allow(dead_code))]
    level: Option<depend::Level>,
    data: UnsafeCell<T>,
}

// SAFETY: 单 guard 持 &mut T；guard !Send（锁在本 hart 释放）
unsafe impl<T: ?Sized> Sync for SpinLock<T> {}

pub struct SpinLockGuard<'a, T: ?Sized> {
    lock: &'a SpinLock<T>,
    _not_send: PhantomData<*const ()>,
    _trap: TrapGuard,
}

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
        let caller = crate::lock::depend_enter!(self);
        // SAFETY: 处于 S-mode；关中断防止本 hart 中断重入
        let trap = unsafe { TrapGuard::save() };
        crate::lock::depend_check!(self, caller);

        while self.locked.swap(true, Ordering::Acquire) {
            crate::runtime::diagnose::halt::hush();
            core::hint::spin_loop();
        }
        self.caller.store(caller, Ordering::Relaxed);
        crate::lock::depend_acquire!(self, caller);

        SpinLockGuard {
            lock: self,
            _not_send: PhantomData,
            _trap: trap,
        }
    }

    #[allow(dead_code)]
    #[inline(never)]
    pub fn try_lock(&self) -> Option<SpinLockGuard<'_, T>> {
        let caller: usize;
        // SAFETY: 读 ra（asm 未声明视为 clobber）
        unsafe { core::arch::asm!("mv {0}, ra", out(reg) caller) };
        // SAFETY: 处于 S-mode；关中断防止本 hart 中断重入
        let trap = unsafe { TrapGuard::save() };

        if self.locked.swap(true, Ordering::Acquire) {
            return None;
        }
        self.caller.store(caller, Ordering::Relaxed);
        crate::lock::depend_acquire!(self, caller);

        Some(SpinLockGuard {
            lock: self,
            _not_send: PhantomData,
            _trap: trap,
        })
    }
}

impl<T: ?Sized> Deref for SpinLockGuard<'_, T> {
    type Target = T;

    fn deref(&self) -> &T {
        // SAFETY: 单 guard 持锁期间无其他持有者
        unsafe { &*self.lock.data.get() }
    }
}

impl<T: ?Sized> DerefMut for SpinLockGuard<'_, T> {
    fn deref_mut(&mut self) -> &mut T {
        // SAFETY: 单 guard 持锁期间无其他持有者
        unsafe { &mut *self.lock.data.get() }
    }
}

impl<T: ?Sized> Drop for SpinLockGuard<'_, T> {
    fn drop(&mut self) {
        crate::lock::depend_release!(self.lock);
        self.lock.locked.store(false, Ordering::Release);
        self.lock.caller.store(0, Ordering::Relaxed);
    }
}