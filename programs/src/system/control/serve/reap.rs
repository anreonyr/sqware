use super::unit::Control;
use crate::system::control::core::{
    unit::{Slot, State, Table},
    verdict::Reaped,
};
use crate::system::identity::serve::install::Roster;
use alloc::{string::String, vec::Vec};
use env::Wait;
use protocol::debug;
use runtime::env::unit as utask;
pub(crate) fn sweep(control: &mut Control, roster: &Roster) {
    let gone: Vec<String> = control
        .table
        .living()
        .filter_map(|row| match row.slot {
            Slot::Live { task, .. } if utask::join(task, Wait::POLL).unwrap_or(true) => {
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
    let before = utask::heir_count();
    let ousted = match team {
        Some(team) => utask::oust(team).is_ok(),
        None => false,
    };
    let after = utask::heir_count();
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
