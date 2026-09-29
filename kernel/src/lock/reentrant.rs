use core::cell::UnsafeCell;
use core::marker::PhantomData;
use core::ops::{Deref, DerefMut};
use core::sync::atomic::{AtomicUsize, Ordering};

use crate::hart::{self, HartId};
use crate::platform::machine;

use super::depend;
use super::trap::TrapGuard;

#[derive(Clone, Copy, PartialEq, Eq)]
struct Owner(usize);

impl Owner {
    const FREE: Owner = Owner(0);

    fn of(hart: HartId) -> Owner {
        Owner(hart.get() + 1)
    }

    fn word(self) -> usize {
        self.0
    }

    fn from_word(w: usize) -> Owner {
        Owner(w)
    }
}

#[derive(Debug)]
pub struct RelLock<T: ?Sized> {
    owner: AtomicUsize,
    count: UnsafeCell<usize>,
    caller: AtomicUsize,
    #[cfg_attr(not(debug_assertions), allow(dead_code))]
    level: Option<depend::Level>,
    data: UnsafeCell<T>,
}

// SAFETY: 同一时刻只有一个 hart 持锁；count 仅在持锁时访问
unsafe impl<T: ?Sized + Send> Sync for RelLock<T> {}

pub struct RelLockGuard<'a, T: ?Sized> {
    lock: &'a RelLock<T>,
    _not_send: PhantomData<*const ()>,
    _trap: TrapGuard,
}

impl<T> RelLock<T> {
    pub const fn new(val: T) -> Self {
        RelLock {
            owner: AtomicUsize::new(0),
            count: UnsafeCell::new(0),
            caller: AtomicUsize::new(0),
            level: None,
            data: UnsafeCell::new(val),
        }
    }

    #[cfg_attr(not(debug_assertions), allow(dead_code))]
    pub const fn new_level(level: depend::Level, val: T) -> Self {
        RelLock {
            owner: AtomicUsize::new(0),
            count: UnsafeCell::new(0),
            caller: AtomicUsize::new(0),
            level: Some(level),
            data: UnsafeCell::new(val),
        }
    }
}

impl<T: ?Sized> RelLock<T> {
    pub(crate) fn read_unlocked(&self) -> *const T {
        self.data.get()
    }

    #[inline(never)]
    pub fn lock(&self) -> RelLockGuard<'_, T> {
        let caller: usize;
        // SAFETY: 读 ra（asm 未声明视为 clobber）
        unsafe { core::arch::asm!("mv {}, ra", out(reg) caller) };
        // SAFETY: 处于 S-mode；关中断防止本 hart 中断重入
        let trap = unsafe { TrapGuard::save() };
        let me = Owner::of(hart::hart_id());

        #[cfg(debug_assertions)]
        if Owner::from_word(self.owner.load(Ordering::Relaxed)) != me {
            depend::check(
                self as *const Self as *const () as usize,
                self.level,
                caller,
            );
        }

        loop {
            match self.owner.compare_exchange(
                Owner::FREE.word(),
                me.word(),
                Ordering::Acquire,
                Ordering::Relaxed,
            ) {
                Ok(_) => {
                    // SAFETY: 刚获独占所有权，count 仅本 hart 访问
                    unsafe { *self.count.get() = 1 };
                    self.caller.store(caller, Ordering::Relaxed);
                    #[cfg(debug_assertions)]
                    depend::acquire(
                        self as *const Self as *const () as usize,
                        self.level,
                        caller,
                    );
                    break;
                }
                Err(cur) if Owner::from_word(cur) == me => {
                    // SAFETY: 本 hart 持锁，count 仅本 hart 访问
                    unsafe { *self.count.get() += 1 };
                    break;
                }
                Err(_) => {
                    crate::runtime::diagnose::halt::hush();
                    core::hint::spin_loop();
                }
            }
        }

        RelLockGuard {
            lock: self,
            _not_send: PhantomData,
            _trap: trap,
        }
    }
}

impl<T: ?Sized> Deref for RelLockGuard<'_, T> {
    type Target = T;

    fn deref(&self) -> &T {
        // SAFETY: 本 hart 持有锁
        unsafe { &*self.lock.data.get() }
    }
}

impl<T: ?Sized> DerefMut for RelLockGuard<'_, T> {
    fn deref_mut(&mut self) -> &mut T {
        // SAFETY: 本 hart 独占持有锁
        unsafe { &mut *self.lock.data.get() }
    }
}

impl<T: ?Sized> Drop for RelLockGuard<'_, T> {
    fn drop(&mut self) {
        // SAFETY: 本 hart 持锁，count 仅本 hart 访问
        let c = unsafe {
            let p = self.lock.count.get();
            *p -= 1;
            *p
        };
        if c == 0 {
            #[cfg(debug_assertions)]
            depend::release(self.lock as *const _ as *const () as usize);
            self.lock.caller.store(0, Ordering::Relaxed);
            self.lock.owner.store(Owner::FREE.word(), Ordering::Release);
        }
    }
}
