use ::core::marker::PhantomData;
use ::core::ops::Deref;
use ::core::sync::atomic::Ordering;

use crate::machine::sync::trap::TrapGuard;

use super::RwLock;

pub struct RwLockReadGuard<'a, T: ?Sized> {
    lock: &'a RwLock<T>,
    _not_send: PhantomData<*const ()>,
    _trap: TrapGuard,
}

impl<'a, T: ?Sized> RwLockReadGuard<'a, T> {
    pub(super) fn new(lock: &'a RwLock<T>, trap: TrapGuard) -> Self {
        RwLockReadGuard {
            lock,
            _not_send: PhantomData,
            _trap: trap,
        }
    }
}

impl<T: ?Sized> Deref for RwLockReadGuard<'_, T> {
    type Target = T;

    fn deref(&self) -> &T {
        unsafe { &*self.lock.data.get() }
    }
}

impl<T: ?Sized> Drop for RwLockReadGuard<'_, T> {
    fn drop(&mut self) {
        self.lock.state.fetch_sub(1, Ordering::Release);
    }
}
