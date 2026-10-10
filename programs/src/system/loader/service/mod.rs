mod answer;
pub(crate) use answer::{Inbox, release_image};
mod execution;
mod plan;
mod publication;
mod resources;
mod watch;

use ::schedule::Resources;
use env::PieToken;

pub(crate) use plan::{frame, shutdown};
pub(crate) use publication::faces;
pub(crate) use resources::resources as install;
pub(crate) use watch::entries as watch;

pub(crate) fn entry(resources: &Resources<'_>) -> Result<PieToken, &'static str> {
    resources
        .read::<answer::Inbox>()
        .map_err(|_| "Loader inbox is not installed")?
        .entry
        .ok_or("Loader service is not published")
}
