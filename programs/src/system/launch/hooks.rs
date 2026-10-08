use crate::system::{
    control::identity::Roster,
    control::instance::hook::{self, Active, Key},
};
use ::schedule::{BuildError, Plan, Progress, Res, Schedule};
pub fn instance() -> Result<Plan<crate::system::app::Fault>, BuildError> {
    hook::plan(children()?)
}
pub fn children() -> Result<alloc::vec::Vec<(Key, Plan<&'static str>)>, BuildError> {
    let mut prepare = Schedule::sequence();
    prepare.system("identity", bind)?;
    prepare.plan("runtime", crate::system::publication::prepare_runtime()?)?;
    prepare.system("runtime.ready", ready)?;
    let mut retire = Schedule::sequence();
    retire.plan("publications", crate::system::publication::retire_tasks()?)?;
    retire.system("identity", unbind)?;
    retire.system("reclaim", hook::reclaim)?;
    Ok(alloc::vec![
        (Key::Prepare, prepare.build()?),
        (Key::Retire, retire.build()?)
    ])
}
fn bind(
    active: Res<Active>,
    pending: Res<super::Pending>,
    roster: Res<Roster>,
) -> Result<Progress, &'static str> {
    let launch = pending
        .0
        .iter()
        .find(|launch| Some(launch.task) == active.task)
        .ok_or("instance installation policy")?;
    roster.install(launch.task, launch.identity)?;
    Ok(Progress::Done)
}
fn ready(
    active: Res<Active>,
    resources: Res<crate::system::publication::RuntimeNamespace>,
) -> Result<Progress, &'static str> {
    let task = active.task.ok_or("instance hook target")?;
    Ok(if resources.runtime_road(task).is_some() {
        Progress::Done
    } else {
        Progress::Pending
    })
}
fn unbind(active: Res<Active>, roster: Res<Roster>) -> Result<Progress, &'static str> {
    roster.unbind(active.task.ok_or("instance hook target")?)?;
    Ok(Progress::Done)
}
