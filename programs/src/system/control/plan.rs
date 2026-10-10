use super::lifecycle::Action;
use super::unit::Control;
use super::{
    endpoint::{self, request as answer},
    lifecycle::{self, Operations},
};
use crate::system::app::Fault as Fail;
use ::schedule::{BuildError, Plan, Progress, Res, ResMut, Schedule};
use env::Wait;
pub(crate) fn poll() -> Result<Plan<Fail>, BuildError> {
    Schedule::sequence()
        .system("reap", super::unit::reap::sweep)?
        .system("instances.receive", endpoint::receive_instances)?
        .system("requests.receive", answer::receive)?
        .system("requests.state", answer::state)?
        .build()
}
pub(crate) fn commands(hooks: super::ActivationHooks) -> Result<Plan<Fail>, BuildError> {
    Schedule::sequence()
        .system("instances.answer", endpoint::answer_instances)?
        .system("requests.enqueue", answer::enqueue)?
        .plan(
            "lifecycle",
            lifecycle::schedule::actions(lifecycle::schedule::lifecycle(hooks)?)?,
        )?
        .build()
}
pub fn reply(mut operations: ResMut<Operations>) -> Result<Progress, Fail> {
    operations.reply_completed();
    Ok(Progress::Done)
}
pub fn pending(
    operations: Res<Operations>,
    inbox: Res<crate::system::control::endpoint::request::Inbox>,
    mut bound: ResMut<crate::system::app::policy::Bound>,
) -> Result<Progress, Fail> {
    if !operations.is_empty() || !inbox.0.is_empty() {
        bound.0 = Wait::AtMost(1);
    }
    Ok(Progress::Done)
}
pub fn retire_completed(mut operations: ResMut<Operations>) -> Result<Progress, Fail> {
    operations.retire_local_completed();
    Ok(Progress::Done)
}

pub fn ruin_rest(
    control: Res<Control>,
    flow: Res<crate::system::app::policy::Flow>,
    mut operations: ResMut<Operations>,
) -> Result<Progress, Fail> {
    // Dynamic tasks may hold resources from any static provider. Their retire
    // hooks reclaim the teams before static services begin to relinquish them.
    if flow.settling && control.instances().all(|item| item.team.is_none()) {
        for name in control.closing_service_names() {
            if !operations.contains(name) {
                operations
                    .submit(name.into(), Action::Ruin)
                    .map_err(|_| Fail::Room)?;
            }
        }
    }
    Ok(Progress::Done)
}
