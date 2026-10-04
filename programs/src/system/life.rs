use core::sync::atomic::{AtomicU8, AtomicUsize, Ordering};
use env::Wait;
use protocol::debug;
use runtime::core::adapt;

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
    status: protocol::common::schedule::Res<alloc::sync::Arc<Status>>,
    mut deadline: protocol::common::schedule::ResMut<Deadline>,
) -> Result<protocol::common::schedule::Progress, crate::system::control::serve::Fail> {
    status.phase.store(Phase::Stopping as u8, Ordering::Release);
    deadline.0 = env::chrono::clock()
        + crate::system::control::serve::start::BOOT_MS as u64 * 1_000_000;
    Ok(protocol::common::schedule::Progress::Done)
}
pub fn join(
    status: protocol::common::schedule::Res<alloc::sync::Arc<Status>>,
    deadline: protocol::common::schedule::Res<Deadline>,
) -> Result<protocol::common::schedule::Progress, crate::system::control::serve::Fail> {
    for task in [
        status.operator.load(Ordering::Acquire),
        status.identity.load(Ordering::Acquire),
    ] {
        if !env::unit::join(env::TaskId::new(task), Wait::POLL).unwrap_or(true) {
            if env::chrono::clock() >= deadline.0 {
                return Err(crate::system::control::serve::Fail::Shutdown);
            }
            adapt::sleep(core::time::Duration::from_millis(1))
                .map_err(|_| crate::system::control::serve::Fail::Wait)?;
            return Ok(protocol::common::schedule::Progress::Pending);
        }
    }
    debug::put("system: internal tasks stopped");
    Ok(protocol::common::schedule::Progress::Done)
}
