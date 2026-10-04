use alloc::boxed::Box;

use crate::hart;
use crate::memory::manager::MapError;
use crate::runtime::switcher::trampoline::restore;

use super::core::{SCHEDULERS, Scheduler};
use super::trap::run;

pub fn init() -> Result<(), MapError> {
    let n = hart::hart_count();
    assert!(n > 0, "no harts");
    let mut sched = Box::<[Scheduler]>::try_new_uninit_slice(n)
        .map_err(|_| MapError::OutOfMemory)?;
    for (h, slot) in sched.iter_mut().enumerate() {
        slot.write(Scheduler::new(hart::HartId::new(h)));
    }
    // SAFETY: every slot has been initialized above.
    let mut sched = unsafe { sched.assume_init() };
    for (h, c) in sched.iter_mut().enumerate() {
        hart::set_scheduler(hart::HartId::new(h), c as *mut Scheduler as *mut ());
    }
    assert!(
        SCHEDULERS.set(Box::leak(sched)).is_ok(),
        "schedulers double init"
    );
    Ok(())
}

pub fn idle() -> ! {
    restore(run())
}
