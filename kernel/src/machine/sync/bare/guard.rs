use ::core::marker::PhantomData;
use ::core::ops::{Deref, DerefMut};
use ::core::sync::atomic::Ordering;

use super::BareLock;

pub struct BareLockGuard<'a, T: ?Sized> {
    lock: &'a BareLock<T>,
    _not_send: PhantomData<*const ()>,
}

impl<'a, T: ?Sized> BareLockGuard<'a, T> {
    pub(super) fn new(lock: &'a BareLock<T>) -> Self {
        BareLockGuard {
            lock,
            _not_send: PhantomData,
        }
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
