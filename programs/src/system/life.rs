use core::sync::atomic::{AtomicU8, AtomicUsize, Ordering};
use env::Wait;
use protocol::debug;
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

pub fn stop(status: &Status) -> Result<(), crate::system::control::serve::Fail> {
    status.phase.store(Phase::Stopping as u8, Ordering::Release);
    let until = runtime::env::chrono::clock()
        + crate::system::control::serve::start::BOOT_MS as u64 * 1_000_000;
    for task in [
        status.operator.load(Ordering::Acquire),
        status.identity.load(Ordering::Acquire),
    ] {
        while !runtime::env::unit::join(env::TaskId::new(task), Wait::POLL).unwrap_or(true) {
            if runtime::env::chrono::clock() >= until {
                let _ = runtime::env::room::doom(status.control);
                return Err(crate::system::control::serve::Fail::Shutdown);
            }
            runtime::env::room::sleep(core::time::Duration::from_millis(1))
                .map_err(|_| crate::system::control::serve::Fail::Wait)?;
        }
    }
    debug::put("system: internal tasks stopped");
    Ok(())
}
