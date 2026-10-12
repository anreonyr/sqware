//! Dynamic Unit resource supply during instance preparation.
use super::Jobs;
use ::schedule::{Progress, ResMut};

pub(super) fn prepare(
    mut jobs: ResMut<Jobs>,
    mut supplies: ResMut<crate::system::app::supplies::Supplies>,
) -> Result<Progress, &'static str> {
    let reference = jobs
        .active_hook_reference()
        .ok_or("instance supply target")?;
    let launch = jobs
        .launches
        .iter_mut()
        .find(|launch| launch.instance.reference == reference)
        .ok_or("instance supply installation")?;
    let Some(program) = launch.installation.delivery.program else {
        return Ok(Progress::Done);
    };
    let Some(service) = launch.installation.service.as_mut() else {
        return if program.supply().is_empty() {
            Ok(Progress::Done)
        } else {
            Err("instance supply service unavailable")
        };
    };
    for binding in program.supply() {
        supplies
            .supply(service, *binding)
            .map_err(|_| "instance resource supply")?;
    }
    Ok(Progress::Done)
}
