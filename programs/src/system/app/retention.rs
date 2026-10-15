use alloc::vec::Vec;
use env::{AcquireSource, FlatReference, PieToken, Wait};
use schedule::{Progress, Res, ResMut};

use crate::system::supply::retention::{Failure, MemoryEffects};

use super::{Fault, policy::Flow, supplies::Supplies};

pub(crate) struct References;

impl MemoryEffects for References {
    fn acquire(
        &mut self,
        source: AcquireSource,
        retention: PieToken,
    ) -> Result<FlatReference, Failure> {
        execution::memory::acquire(source, retention).map_err(|_| Failure::Denied)
    }

    fn release(&mut self, reference: FlatReference) -> Result<(), Failure> {
        execution::memory::release(reference).map_err(|_| Failure::Pending)
    }
}

pub(crate) fn closing(flow: Res<Flow>, mut supplies: ResMut<Supplies>) -> Result<Progress, Fault> {
    if flow.settling {
        supplies.retention.close_all();
    }
    Ok(Progress::Done)
}

pub(crate) fn sweep(mut supplies: ResMut<Supplies>) -> Result<Progress, Fault> {
    let controllers: Vec<_> = supplies.retention.controllers().collect();
    for controller in controllers {
        if matches!(env::unit::join_task(controller, Wait::POLL), Ok(true)) {
            supplies.retention.offline(controller);
        }
    }
    if supplies.retention.retry(&mut References) {
        report(&supplies);
    }
    Ok(Progress::Done)
}

pub(crate) fn report(supplies: &Supplies) {
    for (range, controller, phase, pages, failure) in supplies.retention.occupied() {
        programs::debug::put(&alloc::format!(
            "system: device memory held base={:#x} length={:#x} tid={} phase={phase:?} references={pages} failure={failure:?}",
            range.base(),
            range.length(),
            controller.get(),
        ));
    }
}

pub(crate) fn reclaim(mut supplies: ResMut<Supplies>) -> Result<Progress, Fault> {
    supplies.retention.close_all();
    if supplies.retention.retry(&mut References) {
        report(&supplies);
    }
    if supplies.retention.occupied().next().is_some() {
        env::room::starve();
        return Ok(Progress::Pending);
    }
    Ok(Progress::Done)
}
