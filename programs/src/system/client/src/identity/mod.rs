mod client;
use system_api::identity::frame::vocab::*;
use system_api::identity::frame::{self, Fail, Reply, Wire};
use system_api::identity::grant::Grant;
use system_api::operator::Path;

const DIR: &Path = Path::new(system_api::identity::DIR);

pub use client::{
    CallError, ExportBinding, Exporter, Face, IdentityExport, Installer, Organization, Query,
    SelfOps, TaskQuery,
};

mod discovery;
pub use discovery::authority;

/// Forward a System-issued owner-local Resolve carrier to the owner's constructed child.
/// Task numbers and marks select this token but do not establish its provenance.
pub fn forward_resolve(
    seed: env::PieToken,
    authority: env::TaskId,
    issuer: env::TaskId,
    child: env::TaskId,
) -> Result<(), CallError> {
    let info = resource::raw::inspect(seed).map_err(|_| CallError::Transport)?;
    let mut allowed = env::Permission::STORE | env::Permission::VEST;
    if info.permission.contains(env::Permission::ONLY) {
        allowed |= env::Permission::ONLY;
    }
    if !info.alive
        || info.owner != authority
        || info.vestor != issuer
        || info.mark != env::Mark::of("identity/resolve-carrier")
        || info.kind != env::PieKind::Hole
        || info.permission != allowed
    {
        return Err(CallError::WrongAuthority);
    }
    env::pie::accord(seed, child, env::Permission::STORE, Grant::Resolve.mark())
        .map(|_| ())
        .map_err(|_| CallError::Service(Fail::Denied))
}
