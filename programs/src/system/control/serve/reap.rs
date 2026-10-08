use super::unit::Control;
use crate::system::control::core::{
    unit::{Slot, State, Table},
    verdict::Reaped,
};
use crate::system::control::identity::Roster;
use alloc::{string::String, vec::Vec};
use env::Wait;
use env::unit;
use programs::debug;
pub(crate) fn sweep(
    mut control: ::schedule::ResMut<Control>,
    roster: ::schedule::Res<Roster>,
    operations: ::schedule::Res<crate::system::control::lifecycle::Operations>,
) -> Result<::schedule::Progress, super::Fail> {
    let gone: Vec<String> = control
        .table
        .living()
        .filter(|row| {
            !operations
                .0
                .iter()
                .any(|job| job.operation.request.name == row.name)
        })
        .filter_map(|row| match row.slot {
            Slot::Live { task, .. } if unit::join(task, Wait::POLL).unwrap_or(true) => {
                Some(row.name.clone())
            }
            _ => None,
        })
        .collect();
    for name in &gone {
        if let Some(task) = control.task(name.as_str()) {
            if let Err(why) = roster.unbind(task) {
                debug::put(&alloc::format!("system: departed {name}: {why}"));
            }
        }
        mark_dead(&mut control.table, name.as_str(), Reaped::Now);
    }
    Ok(::schedule::Progress::Done)
}
fn mark_dead(table: &mut Table, name: &str, reaped: Reaped) {
    let Some(row) = table.find(name) else {
        return;
    };
    if matches!(row.state, State::Dead) {
        return;
    }
    let Slot::Live { team, .. } = row.slot else {
        return;
    };
    table.set_state(name, State::Dead);
    let before = unit::heir_count();
    let ousted = match team {
        Some(team) => unit::oust(team).is_ok(),
        None => false,
    };
    let after = unit::heir_count();
    let wait = match reaped {
        Reaped::Now => "now",
        Reaped::Waited => "waited",
        Reaped::Unsettled => "unsettled",
    };
    debug::put(&alloc::format!(
        "system: gone {} state=Dead ousted={ousted} heir={before}→{after} wait={wait}{}",
        name,
        if team.is_none() { " inner" } else { "" }
    ));
}
