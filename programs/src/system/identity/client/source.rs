use crate::system::common::timing::{BOOT_MS, RETRY_MS};
use core::time::Duration;
use env::{PieToken, TaskId};
use ipc::session::establish;
use protocol::system::identity::Grant;
use env::unit;
use ::resource::raw::{pies, reserve};

/// Obtain the startup authority from a Control-injected face, not a marked public entry.
/// Kernel Sire and vestor anchor the trust decision; the original owner is the Identity
/// instance. Discovery clients then verify every other face against that exact owner.
pub fn authority() -> Option<TaskId> {
    let sire = unit::sire();
    let mut authority = None;
    for pie in pies() {
        if pie.mark != Grant::Resolve.mark() {
            continue;
        }
        let Ok((vestor, owner, mark)) = reserve(pie.token) else {
            continue;
        };
        if vestor != sire || owner == sire || mark != Grant::Resolve.mark() {
            continue;
        }
        if authority.is_some_and(|known| known != owner) {
            return None;
        }
        authority = Some(owner);
    }
    authority
}

pub(crate) fn face_of(authority: TaskId, grant: Grant) -> Result<PieToken, &'static str> {
    let mut left = BOOT_MS;
    loop {
        if let Some(entry) = establish::find(authority, grant.mark()) {
            if matches!(reserve(entry), Ok((_, owner, mark))
                if owner == authority && mark == grant.mark())
            {
                return Ok(entry);
            }
            return Err("identity face source");
        }
        if left == 0 {
            return Err("identity face missing");
        }
        execution::room::park(Duration::from_millis(RETRY_MS as u64)).map_err(|_| "identity wait")?;
        left = left.saturating_sub(RETRY_MS);
    }
}
