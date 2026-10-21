use super::Control;
use crate::system::control::{lifecycle::Action, unit::table::Slot};
use alloc::vec::Vec;
use env::{Wait, unit};

pub(crate) fn sweep(
    control: ::schedule::Res<Control>,
    mut jobs: ::schedule::ResMut<crate::system::assembly::Jobs>,
) -> Result<::schedule::Progress, crate::system::app::Fault> {
    let gone: Vec<_> = control
        .table
        .living()
        .filter(|row| !jobs.contains(&row.name))
        .filter_map(|row| match row.slot {
            Slot::Live { task, .. } if unit::join_task(task, Wait::POLL).unwrap_or(true) => {
                Some(row.name.clone())
            }
            _ => None,
        })
        .collect();
    for name in gone {
        // Natural exit uses the same recoverable cleanup as an explicit retirement.
        if jobs.submit_lifecycle(name, Action::Ruin).is_err() {
            return Ok(::schedule::Progress::Pending);
        }
    }
    Ok(::schedule::Progress::Done)
}

pub(crate) fn sweep_images(
    mut supplies: ::schedule::ResMut<crate::system::app::supplies::Supplies>,
) -> Result<::schedule::Progress, crate::system::app::Fault> {
    supplies.sweep_images();
    Ok(::schedule::Progress::Done)
}
