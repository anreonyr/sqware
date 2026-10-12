mod answer;
mod claims;
pub(crate) use answer::release_image;
mod execution;
mod namespace;
mod plan;
mod resources;
mod watch;

use ::schedule::Resources;
use env::PieToken;

pub(crate) use namespace::EndpointState;
pub(crate) use namespace::faces;
pub(crate) use plan::{frame, shutdown};
pub(crate) use resources::resources as install;
pub(crate) use watch::entries as watch;

pub(crate) fn entry(resources: &Resources<'_>) -> Result<PieToken, &'static str> {
    resources
        .read::<namespace::EndpointState>()
        .map_err(|_| "Loader inbox is not installed")?
        .inbox
        .entry
        .ok_or("Loader Claim entry is not installed")
}
