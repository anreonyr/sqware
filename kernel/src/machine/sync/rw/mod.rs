use ::core::cell::UnsafeCell;
use ::core::sync::atomic::{AtomicUsize, Ordering};

use super::depend;
use super::trap::TrapGuard;

mod read_guard;
mod write_guard;

pub use read_guard::RwLockReadGuard;
pub use write_guard::RwLockWriteGuard;

const WRITER_BIT: usize = 1 << (usize::BITS - 1);
const READER_MASK: usize = !WRITER_BIT;

pub struct RwLock<T: ?Sized> {
    state: AtomicUsize,
    caller: AtomicUsize,
    data: UnsafeCell<T>,
}

unsafe impl<T: ?Sized + Send + Sync> Sync for RwLock<T> {}

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
        unsafe { ::core::arch::asm!("mv {}, ra", out(reg) caller) };
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

        RwLockReadGuard::new(self, trap)
    }

    #[inline(never)]
    pub fn write(&self) -> RwLockWriteGuard<'_, T> {
        let caller: usize;
        unsafe { ::core::arch::asm!("mv {}, ra", out(reg) caller) };
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
            ::core::hint::spin_loop();
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

        RwLockWriteGuard::new(self, trap)
    }
}
