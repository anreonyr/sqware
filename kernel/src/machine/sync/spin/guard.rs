use ::core::marker::PhantomData;
use ::core::ops::{Deref, DerefMut};
use ::core::sync::atomic::Ordering;

use crate::machine::sync::trap::TrapGuard;

use super::SpinLock;

pub struct SpinLockGuard<'a, T: ?Sized> {
    lock: &'a SpinLock<T>,
    _not_send: PhantomData<*const ()>,
    _trap: TrapGuard,
}

impl<'a, T: ?Sized> SpinLockGuard<'a, T> {
    pub(super) fn new(lock: &'a SpinLock<T>, trap: TrapGuard) -> Self {
        SpinLockGuard {
            lock,
            _not_send: PhantomData,
            _trap: trap,
        }
    }
}

impl<T: ?Sized> Deref for SpinLockGuard<'_, T> {
    type Target = T;

    fn deref(&self) -> &T {
        unsafe { &*self.lock.data.get() }
    }
}

impl<T: ?Sized> DerefMut for SpinLockGuard<'_, T> {
    fn deref_mut(&mut self) -> &mut T {
        unsafe { &mut *self.lock.data.get() }
    }
}

impl<T: ?Sized> Drop for SpinLockGuard<'_, T> {
    fn drop(&mut self) {
        crate::machine::sync::depend_release!(self.lock);
        self.lock.locked.store(false, Ordering::Release);
        self.lock.caller.store(0, Ordering::Relaxed);
    }
}
