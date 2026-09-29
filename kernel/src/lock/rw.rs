use core::cell::UnsafeCell;
use core::marker::PhantomData;
use core::ops::{Deref, DerefMut};
use core::sync::atomic::{AtomicUsize, Ordering};

use super::depend;
use super::trap::TrapGuard;

const WRITER_BIT: usize = 1 << (usize::BITS - 1);
const READER_MASK: usize = !WRITER_BIT;

pub struct RwLock<T: ?Sized> {
    state: AtomicUsize,
    caller: AtomicUsize,
    data: UnsafeCell<T>,
}

// SAFETY: 读者需 T: Sync；写者需 T: Send
unsafe impl<T: ?Sized + Send + Sync> Sync for RwLock<T> {}

pub struct RwLockReadGuard<'a, T: ?Sized> {
    lock: &'a RwLock<T>,
    _not_send: PhantomData<*const ()>,
    _trap: TrapGuard,
}

pub struct RwLockWriteGuard<'a, T: ?Sized> {
    lock: &'a RwLock<T>,
    _not_send: PhantomData<*const ()>,
    _trap: TrapGuard,
}

impl<T> RwLock<T> {
    pub const fn new(val: T) -> Self {
        RwLock {
            state: AtomicUsize::new(0),
            caller: AtomicUsize::new(0),
            data: UnsafeCell::new(val),
        }
    }
}

impl<T: ?Sized> RwLock<T> {
    #[inline(never)]
    pub fn read(&self) -> RwLockReadGuard<'_, T> {
        let caller: usize;
        // SAFETY: 读 ra（asm 未声明视为 clobber）
        unsafe { core::arch::asm!("mv {}, ra", out(reg) caller) };
        // SAFETY: 处于 S-mode；关中断防止本 hart 中断重入
        let trap = unsafe { TrapGuard::save() };

        let s = self.state.fetch_add(1, Ordering::Acquire);
        if s & WRITER_BIT != 0 {
            self.state.fetch_sub(1, Ordering::Release);
            #[cfg(debug_assertions)]
            depend::report(
                "write->read downgrade deadlock",
                self as *const Self as *const () as usize,
                caller,
            );
            #[cfg(not(debug_assertions))]
            panic!("[rwlock] write->read downgrade deadlock");
        }

        RwLockReadGuard {
            lock: self,
            _not_send: PhantomData,
            _trap: trap,
        }
    }

    #[inline(never)]
    pub fn write(&self) -> RwLockWriteGuard<'_, T> {
        let caller: usize;
        // SAFETY: 读 ra（asm 未声明视为 clobber）
        unsafe { core::arch::asm!("mv {}, ra", out(reg) caller) };
        // SAFETY: 处于 S-mode；关中断防止本 hart 中断重入
        let trap = unsafe { TrapGuard::save() };

        loop {
            let s = self.state.load(Ordering::Relaxed);
            if s & WRITER_BIT != 0 {
                #[cfg(debug_assertions)]
                depend::report(
                    "recursive write acquisition",
                    self as *const Self as *const () as usize,
                    caller,
                );
                #[cfg(not(debug_assertions))]
                panic!("[rwlock] recursive write acquisition");
            }
            if self
                .state
                .compare_exchange(s, s | WRITER_BIT, Ordering::Acquire, Ordering::Relaxed)
                .is_ok()
            {
                break;
            }
            core::hint::spin_loop();
        }

        if self.state.load(Ordering::Acquire) & READER_MASK != 0 {
            #[cfg(debug_assertions)]
            depend::report(
                "read->write upgrade deadlock",
                self as *const Self as *const () as usize,
                caller,
            );
            #[cfg(not(debug_assertions))]
            panic!("[rwlock] read->write upgrade deadlock");
        }
        self.caller.store(caller, Ordering::Relaxed);

        RwLockWriteGuard {
            lock: self,
            _not_send: PhantomData,
            _trap: trap,
        }
    }
}

impl<T: ?Sized> Deref for RwLockReadGuard<'_, T> {
    type Target = T;

    fn deref(&self) -> &T {
        // SAFETY: 持读锁期间无写者
        unsafe { &*self.lock.data.get() }
    }
}

impl<T: ?Sized> Drop for RwLockReadGuard<'_, T> {
    fn drop(&mut self) {
        self.lock.state.fetch_sub(1, Ordering::Release);
    }
}

impl<T: ?Sized> Deref for RwLockWriteGuard<'_, T> {
    type Target = T;

    fn deref(&self) -> &T {
        // SAFETY: 写锁独占
        unsafe { &*self.lock.data.get() }
    }
}

impl<T: ?Sized> DerefMut for RwLockWriteGuard<'_, T> {
    fn deref_mut(&mut self) -> &mut T {
        // SAFETY: 写锁独占
        unsafe { &mut *self.lock.data.get() }
    }
}

impl<T: ?Sized> Drop for RwLockWriteGuard<'_, T> {
    fn drop(&mut self) {
        self.lock.caller.store(0, Ordering::Relaxed);
        self.lock.state.store(0, Ordering::Release);
    }
}
