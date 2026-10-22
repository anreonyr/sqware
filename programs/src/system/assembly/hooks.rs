use super::{Delivery, Jobs};
use crate::system::control::{
    identity::Roster,
    instance::hook::{self, Key},
};
use ::schedule::{BuildError, Plan, Progress, Res, ResMut, Schedule};

pub(crate) fn instance() -> Result<Plan<crate::system::app::Fault>, BuildError> {
    hook::plan(children()?)
}
pub(crate) fn children() -> Result<alloc::vec::Vec<(Key, Plan<&'static str>)>, BuildError> {
    let mut prepare = Schedule::sequence();
    prepare.system("identity", bind)?;
    prepare.system("constructor", constructor)?;
    prepare.system("claim", claim_grant)?;
    prepare.system("namespace.coordinates", namespace_coordinates)?;
    prepare.system("namespace", prepare_namespace)?;
    prepare.system("identity.exports", identity_exports)?;
    prepare.system("supply", super::supply::prepare)?;
    prepare.system("bootstrap", bootstrap)?;
    let mut retire = Schedule::sequence();
    retire.system("retirement", retirement_dependencies)?;
    retire.system("identity.exports", retire_exports)?;
    retire.system("namespace", retire_namespace)?;
    retire.system("constructor", retire_constructor)?;
    retire.system("claim", retire_claim)?;
    retire.system("supply", release_supply)?;
    retire.system("identity", unbind)?;
    retire.system("bootstrap", release_bootstrap)?;
    retire.system("reclaim", hook::reclaim)?;
    Ok(alloc::vec![
        (Key::Prepare, prepare.build()?),
        (Key::Retire, retire.build()?)
    ])
}
fn retirement_dependencies(
    mut jobs: ResMut<Jobs>,
    layout: Res<crate::system::app::operator::Layout>,
    namespaces: Res<crate::system::assembly::namespace::NamespaceInstallations>,
) -> Result<Progress, &'static str> {
    let reference = jobs
        .active_hook_reference()
        .ok_or("instance retirement reference")?;
    let launch = jobs
        .launches
        .iter_mut()
        .find(|launch| launch.instance.reference == reference)
        .ok_or("instance retirement launch")?;
    if launch.installation.retirement.is_empty() {
        return Ok(Progress::Done);
    }
    let system = layout.as_ref().ok_or("instance retirement layout")?;
    for participant in &mut launch.installation.retirement {
        participant.discover(system, &namespaces);
        participant.advance();
    }
    Ok(if launch
        .installation
        .retirement
        .iter()
        .all(|participant| participant.done)
    {
        Progress::Done
    } else {
        Progress::Pending
    })
}

fn release_supply(
    jobs: Res<Jobs>,
    mut supplies: ResMut<crate::system::app::supplies::Supplies>,
) -> Result<Progress, &'static str> {
    let task = jobs.active_hook_task().ok_or("instance hook target")?;
    supplies
        .release_target(task)
        .map_err(|_| "instance supply release")?;
    Ok(Progress::Done)
}
fn bootstrap(mut jobs: ResMut<Jobs>) -> Result<Progress, &'static str> {
    super::bootstrap::advance(&mut jobs, env::unit::self_id())
}
fn release_bootstrap(mut jobs: ResMut<Jobs>) -> Result<Progress, &'static str> {
    super::bootstrap::release(&mut jobs)?;
    Ok(Progress::Done)
}
fn launch(jobs: &Jobs, task: Option<env::TaskId>) -> Result<&Delivery, &'static str> {
    let reference = jobs
        .active_hook_reference()
        .filter(|reference| Some(reference.task) == task)
        .ok_or("instance installation reference")?;
    jobs.launches
        .iter()
        .find(|item| item.instance.reference == reference)
        .map(|item| &item.installation.delivery)
        .ok_or("instance installation policy")
}
fn launch_mut(jobs: &mut Jobs) -> Result<&mut super::Launch, &'static str> {
    let reference = jobs
        .active_hook_reference()
        .ok_or("instance installation reference")?;
    jobs.launches
        .iter_mut()
        .find(|item| item.instance.reference == reference)
        .ok_or("instance installation policy")
}
fn bind(
    mut jobs: ResMut<Jobs>,
    mut roster: ResMut<Roster>,
    control: Res<crate::system::control::unit::Control>,
) -> Result<Progress, &'static str> {
    let task = jobs.active_hook_task().ok_or("instance hook target")?;
    let delivery = launch(&jobs, Some(task))?;
    let owner = delivery.owner;
    if control
        .instance(task)
        .is_none_or(|instance| instance.owner != owner)
    {
        return Err("instance identity owner mismatch");
    }
    let id = roster.install_owned(task, delivery.identity)?;
    let resolve = roster.forward_resolve(crate::system::control::identity::ForwardRequest {
        id,
        task,
        owner,
    })?;
    let launch = launch_mut(&mut jobs)?;
    if launch.instance.task != task {
        return Err("instance installation reference mismatch");
    }
    launch.installation.identity = Some(id);
    launch.installation.startup.resolve = resolve;
    Ok(Progress::Done)
}
fn prepare_namespace(
    jobs: Res<Jobs>,
    layout: Res<crate::system::app::operator::Layout>,
    mut namespaces: ResMut<super::namespace::NamespaceInstallations>,
) -> Result<Progress, &'static str> {
    let task = jobs.active_hook_task().ok_or("instance hook target")?;
    let delivery = launch(&jobs, Some(task))?;
    let system = layout.as_ref().ok_or("operator layout unavailable")?;
    let done = namespaces
        .prepare_task(
            system,
            super::namespace::PrepareTask {
                task,
                program: delivery.program,
                wait: env::Wait::AtMost(crate::support::timing::BOOT_MS),
            },
        )
        .map_err(|_| "instance namespace prepare")?;
    Ok(if done {
        Progress::Done
    } else {
        Progress::Pending
    })
}
fn namespace_coordinates(
    jobs: Res<Jobs>,
    control: Res<crate::system::control::unit::Control>,
    mut namespaces: ResMut<super::namespace::NamespaceInstallations>,
) -> Result<Progress, &'static str> {
    let task = jobs.active_hook_task().ok_or("instance hook target")?;
    let team = control.instance(task).and_then(|instance| instance.team());
    namespaces
        .coordinates(super::namespace::Coordinates { task, team })
        .map_err(|_| "instance namespace coordinates")?;
    Ok(Progress::Done)
}
fn retire_namespace(
    jobs: Res<Jobs>,
    layout: Res<crate::system::app::operator::Layout>,
    mut namespaces: ResMut<super::namespace::NamespaceInstallations>,
) -> Result<Progress, &'static str> {
    let task = jobs.active_hook_task().ok_or("instance hook target")?;
    let system = layout.as_ref().ok_or("operator layout unavailable")?;
    let done = namespaces
        .retire_task(system, task)
        .map_err(|_| "instance namespace retire")?;
    Ok(if done {
        Progress::Done
    } else {
        Progress::Pending
    })
}
fn retire_constructor(
    mut jobs: ResMut<Jobs>,
    mut construction: ResMut<crate::system::control::Construction>,
) -> Result<Progress, &'static str> {
    let task = jobs.active_hook_task().ok_or("instance hook target")?;
    construction.retire(task, |entry, caller| {
        jobs.retire_create_scope(entry, caller).map_err(|_| ())
    })?;
    Ok(Progress::Done)
}
fn unbind(jobs: Res<Jobs>, mut roster: ResMut<Roster>) -> Result<Progress, &'static str> {
    let task = jobs.active_hook_task().ok_or("instance hook target")?;
    let id = jobs
        .launches
        .iter()
        .find(|launch| Some(launch.instance.reference) == jobs.active_hook_reference())
        .and_then(|launch| launch.installation.identity)
        .or_else(|| roster.install_id(task));
    let Some(id) = id else {
        return Ok(Progress::Done);
    };
    roster.retire_install(id)?;
    Ok(Progress::Done)
}
fn constructor(
    jobs: Res<Jobs>,
    mut construction: ResMut<crate::system::control::Construction>,
) -> Result<Progress, &'static str> {
    let task = jobs.active_hook_task().ok_or("instance hook target")?;
    let delivery = launch(&jobs, Some(task))?;
    if let Some(program) = delivery.program {
        construction.grant(task, program.control.construction)?;
    }
    Ok(Progress::Done)
}

fn identity_exports(
    jobs: Res<Jobs>,
    namespaces: Res<super::namespace::NamespaceInstallations>,
    mut exports: ResMut<crate::system::identity::export::Exports>,
) -> Result<Progress, &'static str> {
    let task = jobs.active_hook_task().ok_or("instance hook target")?;
    if let Some(program) = launch(&jobs, Some(task))?.program {
        exports.grant(crate::system::identity::export::GrantRequest {
            task,
            program,
            namespaces: &namespaces,
        })?;
    }
    Ok(Progress::Done)
}
fn retire_exports(
    jobs: Res<Jobs>,
    mut exports: ResMut<crate::system::identity::export::Exports>,
) -> Result<Progress, &'static str> {
    let task = jobs.active_hook_task().ok_or("instance hook target")?;
    exports.retire(task)?;
    Ok(Progress::Done)
}

fn claim_grant(
    jobs: Res<Jobs>,
    mut endpoint: ResMut<crate::system::loader::EndpointState>,
) -> Result<Progress, &'static str> {
    let task = jobs.active_hook_task().ok_or("instance hook target")?;
    if let Some(program) = launch(&jobs, Some(task))?.program {
        endpoint.grant_claim(task, program.control.construction)?;
    }
    Ok(Progress::Done)
}
fn retire_claim(
    jobs: Res<Jobs>,
    mut endpoint: ResMut<crate::system::loader::EndpointState>,
) -> Result<Progress, &'static str> {
    let task = jobs.active_hook_task().ok_or("instance hook target")?;
    endpoint.retire_claim(task)?;
    Ok(Progress::Done)
}
