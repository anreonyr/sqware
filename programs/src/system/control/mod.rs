pub(crate) mod identity;
pub(crate) mod instance;
pub(crate) mod lifecycle;
mod endpoint;
pub(crate) mod unit;

pub(crate) use endpoint::construction::{Construction, receive as receive_construction, admit as admit_construction};

mod install;
pub(crate) use install::{Configuration, install};

mod plan;
pub(crate) use plan::{commands, pending, poll, reply, retire_completed};

pub(crate) fn entry(
    resources: &::schedule::Resources<'_>,
    grant: system_api::control::Grant,
) -> Result<env::PieToken, &'static str> {
    resources
        .read::<endpoint::Entries>()
        .map_err(|_| "Control service not installed")?
        .face(grant)
        .ok_or("Control entry not published")
}
pub(crate) fn instance_entry(
    resources: &::schedule::Resources<'_>,
) -> Result<env::PieToken, &'static str> {
    resources
        .read::<endpoint::Entries>()
        .map_err(|_| "Control service not installed")?
        .instance()
        .ok_or("Control instance entry not published")
}

pub(crate) use lifecycle::schedule::ActivationHooks;

pub(crate) use lifecycle::{Startup, eligibility, startup};
pub(crate) use plan::ruin_rest;

pub(crate) use endpoint::request::receive;
pub(crate) use endpoint::{answer_instances, receive_instances};
pub(crate) use endpoint::Entries as Entries;
