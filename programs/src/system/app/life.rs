use core::sync::atomic::{AtomicU8, AtomicUsize, Ordering};
use env::Wait;
use programs::debug;

#[repr(u8)]
#[derive(Clone, Copy)]
pub enum Phase {
    Starting,
    Running,
    Stopping,
}

pub struct Status {
    pub(crate) control: env::TaskId,
    pub(crate) operator: AtomicUsize,
    pub(crate) identity: AtomicUsize,
    pub(crate) phase: AtomicU8,
}

pub struct Deadline(pub u64);
pub fn stopping(
    status: ::schedule::Res<alloc::sync::Arc<Status>>,
    mut deadline: ::schedule::ResMut<Deadline>,
) -> Result<::schedule::Progress, crate::system::app::Fault> {
    status.phase.store(Phase::Stopping as u8, Ordering::Release);
    deadline.0 =
        env::chrono::clock() + crate::system::control::unit::start::BOOT_MS as u64 * 1_000_000;
    Ok(::schedule::Progress::Done)
}
pub fn join(
    status: ::schedule::Res<alloc::sync::Arc<Status>>,
    deadline: ::schedule::Res<Deadline>,
) -> Result<::schedule::Progress, crate::system::app::Fault> {
    for task in [
        status.operator.load(Ordering::Acquire),
        status.identity.load(Ordering::Acquire),
    ] {
        if !env::unit::join_task(env::TaskId::new(task), Wait::POLL).unwrap_or(true) {
            if env::chrono::clock() >= deadline.0 {
                return Err(crate::system::app::Fault::Shutdown);
            }
            execution::room::park(core::time::Duration::from_millis(1))
                .map_err(|_| crate::system::app::Fault::Wait)?;
            return Ok(::schedule::Progress::Pending);
        }
    }
    debug::put("system: internal tasks stopped");
    Ok(::schedule::Progress::Done)
}
