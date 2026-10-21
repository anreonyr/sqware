use super::lifecycle::Action;
use super::unit::Control;
use super::{
    endpoint::{self, request as answer},
    lifecycle,
};
use crate::system::app::Fault as Fail;
use crate::system::app::policy::Settle;
use ::schedule::{BuildError, Plan, Progress, Res, ResMut, Schedule};
use env::Wait;
pub(crate) fn poll() -> Result<Plan<Fail>, BuildError> {
    Schedule::sequence()
        .system("supply.reap", super::unit::reap::sweep_images)?
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
pub fn reply(mut operations: ResMut<crate::system::assembly::Jobs>) -> Result<Progress, Fail> {
    operations.reply_completed();
    Ok(Progress::Done)
}
pub fn pending(
    operations: Res<crate::system::assembly::Jobs>,
    inbox: Res<crate::system::control::endpoint::request::Inbox>,
    mut bound: ResMut<crate::system::app::policy::Bound>,
) -> Result<Progress, Fail> {
    if !operations.is_empty() || !inbox.0.is_empty() {
        bound.0 = Wait::AtMost(1);
    }
    Ok(Progress::Done)
}
pub fn retire_completed(
    mut operations: ResMut<crate::system::assembly::Jobs>,
) -> Result<Progress, Fail> {
    operations.retire_local_completed();
    Ok(Progress::Done)
}

pub fn ruin_rest(
    control: Res<Control>,
    settle: Res<Settle>,
    mut operations: ResMut<crate::system::assembly::Jobs>,
) -> Result<Progress, Fail> {
    // Dynamic tasks may hold resources from any static provider. Their retire
    // hooks reclaim the teams before static services begin to relinquish them.
    // 退场门在 `policy::settle` 那一侧：只有 Hub 把全部 DMA 绑定结清之后 settling 才为真，
    // 故这里递的 Ruin 不会抢在恢复责任结清之前拆掉恢复侧。
    if settle.flow.settling && control.instances().all(|item| item.team.is_none()) {
        for name in control.closing_service_names() {
            if !operations.contains(name) {
                operations
                    .submit_lifecycle(name.into(), Action::Ruin)
                    .map_err(|_| Fail::Room)?;
            }
        }
    }
    Ok(Progress::Done)
}
