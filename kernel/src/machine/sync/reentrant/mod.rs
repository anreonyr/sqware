use ::core::cell::UnsafeCell;
use ::core::sync::atomic::{AtomicUsize, Ordering};

use crate::machine::hart;

use super::depend;
use super::trap::TrapGuard;

mod guard;
mod owner;

pub use guard::RelLockGuard;

use owner::Owner;

#[derive(Debug)]
pub struct RelLock<T: ?Sized> {
    owner: AtomicUsize,
    count: UnsafeCell<usize>,
    caller: AtomicUsize,
    #[cfg_attr(not(debug_assertions), allow(dead_code))]
    level: Option<depend::Level>,
    data: UnsafeCell<T>,
}

unsafe impl<T: ?Sized + Send> Sync for RelLock<T> {}

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
    #[cfg(debug_assertions)]
    pub(crate) fn held_by_current(&self) -> bool {
        self.owner.load(Ordering::Relaxed) == Owner::of(hart::hart_id()).word()
    }

    pub(crate) fn read_unlocked(&self) -> *const T {
        self.data.get()
    }

    #[inline(never)]
    pub fn lock(&self) -> RelLockGuard<'_, T> {
        let caller: usize;
        unsafe { ::core::arch::asm!("mv {}, ra", out(reg) caller) };
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
                    unsafe { *self.count.get() += 1 };
                    break;
                }
                Err(_) => {
                    crate::machine::stop::hush();
                    ::core::hint::spin_loop();
                }
            }
        }

        RelLockGuard::new(self, trap)
    }
}
