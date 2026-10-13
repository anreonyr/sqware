use ::core::marker::PhantomData;
use ::core::ops::{Deref, DerefMut};
use ::core::sync::atomic::Ordering;

use crate::machine::sync::trap::TrapGuard;

use super::owner::Owner;
use super::RelLock;

pub struct RelLockGuard<'a, T: ?Sized> {
    lock: &'a RelLock<T>,
    _not_send: PhantomData<*const ()>,
    _trap: TrapGuard,
}

impl<'a, T: ?Sized> RelLockGuard<'a, T> {
    pub(super) fn new(lock: &'a RelLock<T>, trap: TrapGuard) -> Self {
        RelLockGuard {
            lock,
            _not_send: PhantomData,
            _trap: trap,
        }
    }
}

impl<T: ?Sized> Deref for RelLockGuard<'_, T> {
    type Target = T;

    fn deref(&self) -> &T {
        unsafe { &*self.lock.data.get() }
    }
}

impl<T: ?Sized> DerefMut for RelLockGuard<'_, T> {
    fn deref_mut(&mut self) -> &mut T {
        unsafe { &mut *self.lock.data.get() }
    }
}

impl<T: ?Sized> Drop for RelLockGuard<'_, T> {
    fn drop(&mut self) {
        let c = unsafe {
            let p = self.lock.count.get();
            *p -= 1;
            *p
        };
        if c == 0 {
            #[cfg(debug_assertions)]
            super::depend::release(self.lock as *const _ as *const () as usize);
            self.lock.caller.store(0, Ordering::Relaxed);
            self.lock.owner.store(Owner::FREE.word(), Ordering::Release);
        }
    }
}
