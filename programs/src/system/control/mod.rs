pub(crate) mod identity;
pub(crate) mod instance;
pub(crate) mod lifecycle;
mod service;
pub(crate) mod unit;

mod install;
pub(crate) use install::{Configuration, install};

mod plan;
pub(crate) use plan::{commands, pending, poll, reply, retire_completed};

pub(crate) fn entry(
    resources: &::schedule::Resources<'_>,
    grant: system_api::control::Grant,
) -> Result<env::PieToken, &'static str> {
    resources
        .read::<service::watch::Watch>()
        .map_err(|_| "Control service not installed")?
        .face(grant)
        .ok_or("Control entry not published")
}
pub(crate) fn instance_entry(
    resources: &::schedule::Resources<'_>,
) -> Result<env::PieToken, &'static str> {
    resources
        .read::<service::watch::Watch>()
        .map_err(|_| "Control service not installed")?
        .instance()
        .ok_or("Control instance entry not published")
}

pub(crate) use lifecycle::schedule::ActivationHooks;

pub(crate) use lifecycle::{Startup, eligibility, startup};
pub(crate) use plan::ruin_rest;

pub(crate) use service::answer::receive;
pub(crate) use service::instance::{answer as answer_instances, receive as receive_instances};
pub(crate) use service::watch::Watch as Entries;
