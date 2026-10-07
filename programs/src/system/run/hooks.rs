use crate::system::{
    control::serve::hook::{self, Active, Key},
    identity::client::install::Roster,
    run::names,
};
use ::schedule::{BuildError, Plan, Progress, Res, Schedule};
pub fn instance() -> Result<Plan<crate::system::control::serve::Fail>, BuildError> {
    hook::plan(children()?)
}
pub fn children() -> Result<alloc::vec::Vec<(Key, Plan<&'static str>)>, BuildError> {
    use crate::system::run::living;
    let mut prepare = Schedule::sequence();
    prepare.system("identity", bind)?;
    prepare.system("runtime.candidates", super::resource::candidates)?;
    prepare.system("runtime.prepare", super::resource::prepare)?;
    prepare.system("runtime.install", super::resource::install)?;
    prepare.system("runtime.ready", ready)?;
    let mut retire = Schedule::sequence();
    retire.system("living", living::capture)?;
    retire.system("authority", living::authority)?;
    retire.system("operator", living::operator)?;
    retire.system("publications", super::publication::retire::retire)?;
    retire.system("names.expired", names::expired)?;
    retire.system("names.retire", names::retire)?;
    retire.system("runtime", super::resource::retire)?;
    retire.system("identity", unbind)?;
    retire.system("reclaim", hook::reclaim)?;
    Ok(alloc::vec![
        (Key::Prepare, prepare.build()?),
        (Key::Retire, retire.build()?)
    ])
}
fn bind(
    active: Res<Active>,
    pending: Res<super::launch::Pending>,
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
    resources: Res<super::resource::Resources>,
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
