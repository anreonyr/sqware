use core::cell::UnsafeCell;
use core::mem::MaybeUninit;
use core::sync::atomic::{AtomicU8, Ordering};

const EMPTY: u8 = 0;
const WRITING: u8 = 1;
const READY: u8 = 2;

pub struct OnceLock<T> {
    state: AtomicU8,
    data: UnsafeCell<MaybeUninit<T>>,
}

unsafe impl<T: Send + Sync> Sync for OnceLock<T> {}

impl<T> OnceLock<T> {
    #[inline]
    pub const fn new() -> Self {
        OnceLock {
            state: AtomicU8::new(EMPTY),
            data: UnsafeCell::new(MaybeUninit::uninit()),
        }
    }

    #[inline]
    pub fn get(&self) -> Option<&T> {
        if self.state.load(Ordering::Acquire) == READY {
            Some(unsafe { (*self.data.get()).assume_init_ref() })
        } else {
            None
        }
    }

    pub fn set(&self, value: T) -> Result<(), T> {
        match self
            .state
            .compare_exchange(EMPTY, WRITING, Ordering::AcqRel, Ordering::Acquire)
        {
            Ok(_) => {
                unsafe { (*self.data.get()).as_mut_ptr().write(value) }
                self.state.store(READY, Ordering::Release);
                Ok(())
            }
            Err(_) => {
                while self.state.load(Ordering::Acquire) == WRITING {
                    core::hint::spin_loop();
                }
                Err(value)
            }
        }
    }

    pub fn get_or_init<F>(&self, f: F) -> &T
    where
        F: FnOnce() -> T,
    {
        if let Some(val) = self.get() {
            return val;
        }
        let val = f();
        if let Err(ours) = self.set(val) {
            drop(ours);
        }
        self.get().unwrap()
    }

    #[inline]
    #[allow(dead_code)]
    pub fn is_initialized(&self) -> bool {
        self.state.load(Ordering::Acquire) == READY
    }
}
