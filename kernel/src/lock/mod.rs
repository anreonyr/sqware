#![allow(unused)]

mod bare;
mod depend;
mod lazy;
mod once;
pub(crate) mod reentrant;
mod rw;
mod spin;
mod trap;

macro_rules! depend_enter {
    ($lock:expr) => {{
        let caller: usize;
        // SAFETY: 读 ra（asm 未声明视为 clobber）
        unsafe { core::arch::asm!("mv {0}, ra", out(reg) caller) };
        caller
    }};
}

macro_rules! depend_check {
    ($lock:expr, $caller:expr) => {{
        #[cfg(debug_assertions)]
        crate::lock::depend::check(
            $lock as *const _ as *const () as usize,
            $lock.level,
            $caller,
        );
    }};
}

macro_rules! depend_acquire {
    ($lock:expr, $caller:expr) => {{
        #[cfg(debug_assertions)]
        crate::lock::depend::acquire(
            $lock as *const _ as *const () as usize,
            $lock.level,
            $caller,
        );
    }};
}

macro_rules! depend_release {
    ($lock:expr) => {{
        #[cfg(debug_assertions)]
        crate::lock::depend::release($lock as *const _ as *const () as usize);
    }};
}
pub(crate) use depend_acquire;
pub(crate) use depend_check;
pub(crate) use depend_enter;
pub(crate) use depend_release;

pub use bare::BareLock;
pub use depend::Level;
pub use once::OnceLock;
pub use reentrant::RelLock;
pub use rw::RwLock;
pub use spin::SpinLock;

#[cfg(debug_assertions)]
pub fn init_depend(hart_count: usize) -> Result<(), depend::DepInitError> {
    depend::init(hart_count)
}
