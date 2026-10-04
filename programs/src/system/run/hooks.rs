use crate::system::{
    control::serve::hook::{self, Active, Key},
    identity::serve::{install::Roster, names},
};
use protocol::common::schedule::{BuildError, Plan, Progress, Res, Schedule};
pub fn instance() -> Result<Plan<crate::system::control::serve::Fail>, BuildError> {
    hook::plan(children()?)
}
pub fn children() -> Result<alloc::vec::Vec<(Key, Plan<&'static str>)>, BuildError> {
    use crate::system::control::serve::living;
    let mut prepare = Schedule::new();
    prepare.add_system("identity", 0u8, bind)?;
    prepare.add_system("runtime.candidates", 1, super::resource::candidates)?;
    prepare.add_system("runtime.prepare", 2, super::resource::prepare)?;
    prepare.add_system("runtime.install", 3, super::resource::install)?;
    prepare.add_system("runtime.ready", 4, ready)?;
    let mut retire = Schedule::new();
    retire.add_system("living", 0u8, living::capture)?;
    retire.add_system("authority", 1, living::authority)?;
    retire.add_system("operator", 2, living::operator)?;
    retire.add_system("publications", 3, super::publication::retire::retire)?;
    retire.add_system("names.expired", 4, names::expired)?;
    retire.add_system("names.retire", 5, names::retire)?;
    retire.add_system("runtime", 6, super::resource::retire)?;
    retire.add_system("identity", 7, unbind)?;
    retire.add_system("reclaim", 8, hook::reclaim)?;
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
