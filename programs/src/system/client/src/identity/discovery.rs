//! Trusted startup authority discovery.

use env::{TaskId, unit};
use resource::raw::{pies, reserve};
use system_api::identity::Grant;

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
