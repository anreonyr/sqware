//! Trusted startup adapters: image selection, capability wiring and supplies.
use crate::system::control::{
    lifecycle::{Active, Instance},
    unit::{Control, verdict::Fail},
};
use ::schedule::{Progress, Res, ResMut};
pub(crate) fn mint(
    mut active: ResMut<Active>,
    mut loader: ResMut<crate::system::loader::Loader>,
    mut control: ResMut<Control>,
) -> Result<Progress, Fail> {
    let job = active.0.as_mut().ok_or(Fail::Unknown)?;
    let input = control.input(&job.key.request.name)?;
    let bytes = input.image.ok_or(Fail::BadImage)?;
    let built = loader
        .construct(
            crate::system::loader::Image {
                bytes,
                kind: input.program.loader.space,
            },
            crate::system::loader::Spawn {
                sire: env::unit::self_id(),
                args: input.program.loader.args,
                stack: input.program.loader.stack,
            },
        )
        .map_err(|fail| match fail {
            system_api::loader::Fail::BadImage => Fail::BadImage,
            _ => Fail::Full,
        })?;
    let service = control
        .attach_service(input.program.name(), built)
        .map_err(|_| Fail::Full)?;
    job.progress.execution.task = Some(service.task());
    job.progress.execution.instance = Some(Instance {
        service,
        launched: false,
        identity_install: None,
    });
    Ok(Progress::Done)
}
fn namespace(
    active: Res<Active>,
    layout: Res<crate::system::app::operator::Layout>,
    mut namespaces: ResMut<super::namespace::NamespaceInstallations>,
) -> Result<Progress, Fail> {
    let job = active.0.as_ref().ok_or(Fail::Unknown)?;
    let task = active.task().ok_or(Fail::NotReady)?;
    let program = crate::unit::PROGRAMS
        .iter()
        .copied()
        .find(|program| program.name() == job.key.request.name)
        .ok_or(Fail::Unknown)?;
    crate::debug::put(&alloc::format!("system: prepare {} namespace", program.name()));
    let system = layout.as_ref().ok_or(Fail::NotReady)?;
    let done = namespaces
        .prepare_task(
            system,
            super::namespace::PrepareTask {
                task,
                program: Some(program),
                wait: env::Wait::AtMost(crate::support::timing::BOOT_MS),
            },
        )
        .map_err(|fail| {
            crate::debug::put(&alloc::format!("system: namespace {} failed {fail:?}", job.key.request.name));
            Fail::NotReady
        })?;
    Ok(if done {
        Progress::Done
    } else {
        Progress::Pending
    })
}
fn namespace_coordinates(
    active: Res<Active>,
    control: Res<Control>,
    mut namespaces: ResMut<super::namespace::NamespaceInstallations>,
) -> Result<Progress, Fail> {
    let task = active.task().ok_or(Fail::NotReady)?;
    namespaces
        .coordinates(super::namespace::Coordinates {
            task,
            team: control.task_team(task),
        })
        .map_err(|_| Fail::NotReady)?;
    Ok(Progress::Done)
}
fn retire_namespace(
    active: Res<Active>,
    layout: Res<crate::system::app::operator::Layout>,
    mut namespaces: ResMut<super::namespace::NamespaceInstallations>,
) -> Result<Progress, Fail> {
    let Some(task) = active.task() else {
        return Ok(Progress::Done);
    };
    let system = layout.as_ref().ok_or(Fail::NotReady)?;
    let done = namespaces
        .retire_task(system, task)
        .map_err(|_| Fail::NotReady)?;
    Ok(if done {
        Progress::Done
    } else {
        Progress::Pending
    })
}
pub(crate) fn prepare(
    active: Res<Active>,
    mut construction: ResMut<crate::system::control::Construction>,
    control: Res<Control>,
) -> Result<Progress, Fail> {
    let job = active.0.as_ref().ok_or(Fail::Unknown)?;
    let policies = control
        .input(&job.key.request.name)?
        .program
        .control
        .construction;
    construction
        .grant(active.task().ok_or(Fail::NotReady)?, policies)
        .map_err(|_| Fail::Full)?;
    Ok(Progress::Done)
}
fn identity_exports(
    active: Res<Active>,
    namespaces: Res<super::namespace::NamespaceInstallations>,
    mut exports: ResMut<crate::system::identity::export::Exports>,
) -> Result<Progress, Fail> {
    let job = active.0.as_ref().ok_or(Fail::Unknown)?;
    let task = active.task().ok_or(Fail::NotReady)?;
    let program = crate::unit::PROGRAMS
        .iter()
        .copied()
        .find(|program| program.name() == job.key.request.name)
        .ok_or(Fail::Unknown)?;
    exports
        .grant(crate::system::identity::export::GrantRequest {
            task,
            program,
            namespaces: &namespaces,
        })
        .map_err(|_| Fail::NotReady)?;
    Ok(Progress::Done)
}
fn retire_exports(
    active: Res<Active>,
    mut exports: ResMut<crate::system::identity::export::Exports>,
) -> Result<Progress, Fail> {
    if let Some(task) = active.task() {
        exports.retire(task).map_err(|_| Fail::NotReady)?;
    }
    Ok(Progress::Done)
}
fn claim_grant(
    active: Res<Active>,
    mut endpoint: ResMut<crate::system::loader::EndpointState>,
) -> Result<Progress, Fail> {
    let job = active.0.as_ref().ok_or(Fail::Unknown)?;
    let task = active.task().ok_or(Fail::NotReady)?;
    let program = crate::unit::PROGRAMS
        .iter()
        .copied()
        .find(|program| program.name() == job.key.request.name)
        .ok_or(Fail::Unknown)?;
    endpoint
        .grant_claim(task, program.control.construction)
        .map_err(|_| Fail::NotReady)?;
    Ok(Progress::Done)
}
fn retire_claim(
    active: Res<Active>,
    mut endpoint: ResMut<crate::system::loader::EndpointState>,
) -> Result<Progress, Fail> {
    if let Some(task) = active.task() {
        endpoint.retire_claim(task).map_err(|_| Fail::NotReady)?;
    }
    Ok(Progress::Done)
}
pub(crate) fn identity_grants(
    active: Res<Active>,
    control: Res<Control>,
    mut roster: ResMut<crate::system::control::identity::Roster>,
) -> Result<Progress, Fail> {
    let job = active.0.as_ref().ok_or(Fail::Unknown)?;
    let task = active.task().ok_or(Fail::NotReady)?;
    for grant in control
        .input(&job.key.request.name)?
        .program
        .identity
        .grants
    {
        roster
            .grant_face(task, *grant)
            .map_err(|_| Fail::NotReady)?;
    }
    Ok(Progress::Done)
}
pub(crate) fn supply(
    mut active: ResMut<Active>,
    mut supplies: ResMut<crate::system::app::supplies::Supplies>,
    control: Res<Control>,
) -> Result<Progress, Fail> {
    let job = active.0.as_mut().ok_or(Fail::Unknown)?;
    if let Some(instance) = job.progress.execution.instance.as_mut() {
        let program = control.input(&job.key.request.name)?.program;
        for binding in program.supply() {
            supplies
                .supply(&mut instance.service, *binding)
                .map_err(|why| {
                    crate::debug::put(&alloc::format!("supply {}: {why:?}", program.name()));
                    Fail::NotReady
                })?;
        }
    }
    Ok(Progress::Done)
}
pub(crate) fn hooks() -> Result<crate::system::control::ActivationHooks, ::schedule::BuildError> {
    let mut construct = ::schedule::Schedule::sequence();
    construct.system("construct", mint)?;
    let mut prepare = ::schedule::Schedule::sequence();
    prepare.system("namespace.coordinates", namespace_coordinates)?;
    prepare.system("namespace", namespace)?;
    prepare.system("identity.exports", identity_exports)?;
    prepare.system("capabilities", self::prepare)?;
    prepare.system("claim", claim_grant)?;
    prepare.system("identity-grants", identity_grants)?;
    let mut supply_plan = ::schedule::Schedule::sequence();
    supply_plan.system("supplies", supply)?;
    let mut retire = ::schedule::Schedule::sequence();
    retire.system("claim", retire_claim)?;
    retire.system("identity.exports", retire_exports)?;
    retire.system("namespace", retire_namespace)?;
    Ok(crate::system::control::ActivationHooks {
        construct: construct.build()?,
        prepare: prepare.build()?,
        supply: supply_plan.build()?,
        retire: retire.build()?,
    })
}
