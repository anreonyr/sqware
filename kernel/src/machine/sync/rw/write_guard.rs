use ::core::marker::PhantomData;
use ::core::ops::{Deref, DerefMut};
use ::core::sync::atomic::Ordering;

use crate::machine::sync::trap::TrapGuard;

use super::RwLock;

pub struct RwLockWriteGuard<'a, T: ?Sized> {
    lock: &'a RwLock<T>,
    _not_send: PhantomData<*const ()>,
    _trap: TrapGuard,
}

impl<'a, T: ?Sized> RwLockWriteGuard<'a, T> {
    pub(super) fn new(lock: &'a RwLock<T>, trap: TrapGuard) -> Self {
        RwLockWriteGuard {
            lock,
            _not_send: PhantomData,
            _trap: trap,
        }
    }
}

impl<T: ?Sized> Deref for RwLockWriteGuard<'_, T> {
    type Target = T;

    fn deref(&self) -> &T {
        unsafe { &*self.lock.data.get() }
    }
}

impl<T: ?Sized> DerefMut for RwLockWriteGuard<'_, T> {
    fn deref_mut(&mut self) -> &mut T {
        unsafe { &mut *self.lock.data.get() }
    }
}

impl<T: ?Sized> Drop for RwLockWriteGuard<'_, T> {
    fn drop(&mut self) {
        self.lock.caller.store(0, Ordering::Relaxed);
        self.lock.state.store(0, Ordering::Release);
    }
}
