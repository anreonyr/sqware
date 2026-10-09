//! Trusted startup adapters: image selection, capability wiring and Hub supplies.
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
    let input = control.input(&job.request.name)?;
    let (bytes, kind) = input.image.ok_or(Fail::BadImage)?;
    let built = loader
        .construct(
            crate::system::loader::Image { bytes, kind },
            crate::system::loader::Spawn {
                args: &[],
                stack: 0,
            },
        )
        .map_err(|fail| match fail {
            system_api::loader::Fail::BadImage => Fail::BadImage,
            _ => Fail::Full,
        })?;
    let service = control
        .attach_service(input.program.name(), built)
        .map_err(|_| Fail::Full)?;
    job.execution.task = Some(service.task());
    job.execution.instance = Some(Instance {
        service,
        marks: alloc::vec::Vec::new(),
        launched: false,
    });
    Ok(Progress::Done)
}
pub(crate) fn inject(
    active: Res<Active>,
    entry: Res<crate::system::publication::Entry>,
) -> Result<Progress, Fail> {
    entry
        .inject(active.task().ok_or(Fail::NotReady)?)
        .map_err(|_| Fail::Full)?;
    Ok(Progress::Done)
}
pub(crate) fn supply(
    mut active: ResMut<Active>,
    mut supplies: ResMut<super::supplies::Supplies>,
    control: Res<Control>,
) -> Result<Progress, Fail> {
    let job = active.0.as_mut().ok_or(Fail::Unknown)?;
    if let Some(instance) = job.execution.instance.as_mut() {
        let program = control.input(&job.request.name)?.program;
        if program.supply().iter().any(|setup| setup.machine()) {
            supplies
                .enroll(&mut instance.service, program)
                .map_err(|_| Fail::NotReady)?;
        }
    }
    Ok(Progress::Done)
}
pub(crate) fn hooks() -> Result<crate::system::control::ActivationHooks, ::schedule::BuildError> {
    let mut construct = ::schedule::Schedule::sequence();
    construct.system("construct", mint)?;
    construct.system("publication", inject)?;
    let mut prepare = ::schedule::Schedule::sequence();
    prepare.system("account.consumers", super::account::consumers)?;
    let mut supply_plan = ::schedule::Schedule::sequence();
    supply_plan.system("hub", supply)?;
    Ok(crate::system::control::ActivationHooks {
        construct: construct.build()?,
        prepare: prepare.build()?,
        supply: supply_plan.build()?,
        retire: ::schedule::Schedule::sequence().build()?,
    })
}
