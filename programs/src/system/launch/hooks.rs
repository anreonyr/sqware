use crate::system::{
    control::identity::Roster,
    control::instance::hook::{self, Active, Key},
};
use ::schedule::{BuildError, Plan, Progress, Res, ResMut, Schedule};
pub fn instance() -> Result<Plan<crate::system::app::Fault>, BuildError> {
    hook::plan(children()?)
}
pub fn children() -> Result<alloc::vec::Vec<(Key, Plan<&'static str>)>, BuildError> {
    let mut prepare = Schedule::sequence();
    prepare.system("identity", bind)?;
    prepare.system("constructor", constructor)?;
    prepare.system("loader", loader)?;
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
        .launches
        .iter()
        .find(|launch| Some(launch.task) == active.task)
        .ok_or("instance installation policy")?;
    roster.install(launch.task, launch.delivery.identity)?;
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

fn constructor(
    active: Res<Active>,
    pending: Res<super::Pending>,
    mut construction: ResMut<crate::system::control::Construction>,
) -> Result<Progress, &'static str> {
    let launch = pending
        .launches
        .iter()
        .find(|launch| Some(launch.task) == active.task)
        .ok_or("instance constructor policy")?;
    if launch.delivery.constructor {
        construction.grant(launch.task)?;
    }
    Ok(Progress::Done)
}
fn loader(
    active: Res<Active>,
    pending: Res<super::Pending>,
    loader: Res<crate::system::loader::Inbox>,
) -> Result<Progress, &'static str> {
    let launch = pending
        .launches
        .iter()
        .find(|launch| Some(launch.task) == active.task)
        .ok_or("instance constructor policy")?;
    if launch.delivery.constructor {
        let entry = loader.entry.ok_or("loader grant missing")?;
        env::pie::accord(
            entry,
            launch.task,
            env::Permission::STORE,
            system_api::loader::Grant::Build.mark(),
        )
        .map_err(|_| "instance loader grant")?;
    }
    Ok(Progress::Done)
}
