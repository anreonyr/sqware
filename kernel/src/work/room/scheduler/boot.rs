use alloc::boxed::Box;

use crate::hart;
use crate::runtime::switcher::trampoline::restore;

use super::core::{SCHEDULERS, Scheduler};
use super::trap::run;

pub fn init() {
    let n = hart::hart_count();
    assert!(n > 0, "no harts");
    let mut sched: Box<[Scheduler]> = (0..n)
        .map(|h| Scheduler::new(hart::HartId::new(h)))
        .collect();
    for (h, c) in sched.iter_mut().enumerate() {
        hart::set_scheduler(hart::HartId::new(h), c as *mut Scheduler as *mut ());
    }
    assert!(
        SCHEDULERS.set(Box::leak(sched)).is_ok(),
        "schedulers double init"
    );
}

pub fn idle() -> ! {
    restore(run())
}
