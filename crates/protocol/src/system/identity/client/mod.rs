//! Capability subsets over one authority, using one-shot reply holes.
//! Discovery requires the trusted assembly authority, not just a matching mark.
mod face;
mod installer;
mod organization;
mod query;
mod self_ops;

pub use face::{CallError, Face};
pub use installer::Installer;
pub use organization::Organization;
pub use query::{Query, TaskQuery};
pub use self_ops::SelfOps;

use super::{PrincipalId, Reply};

fn unit(reply: Reply) -> Result<(), CallError> {
    match reply {
        Reply::Unit => Ok(()),
        _ => Err(CallError::Malformed),
    }
}
fn flag(reply: Reply) -> Result<bool, CallError> {
    match reply {
        Reply::Bool(b) => Ok(b),
        _ => Err(CallError::Malformed),
    }
}
fn principal(reply: Reply) -> Result<PrincipalId, CallError> {
    match reply {
        Reply::Principal(Some(p)) => Ok(p),
        _ => Err(CallError::Malformed),
    }
}

pub type IdentityQuery = Query;
pub type IdentitySelf = SelfOps;
pub type IdentityOrganization = Organization;
pub type IdentityInstaller = Installer;
