use crate::service::hub::bridge::Activation;
use crate::system::common::machine::Machine;
use crate::system::control::{
    core::{
        publication::Publications,
        unit::{Slot, State},
    },
    serve::{resource::Resources, unit::Control},
};
use crate::system::identity::serve::{install::Roster, names::Names, query::current_authority};
use crate::system::operator::serve::install::Tree;
use alloc::vec::Vec;
use env::{PieToken, TaskId, Wait};

pub fn poll(
    control: &Control,
    roster: &Roster,
    machine: &Machine,
    activation: &Option<Activation>,
    entry: PieToken,
    publications: &mut Publications,
    resources: &mut Resources,
    names: &mut Names,
    tree: &mut Tree,
) -> Result<(), &'static str> {
    if let Some(activation) = activation {
        activation.poll(control, roster);
    }
    tree.connect(control.tasks())?;
    let mut living = Vec::new();
    living
        .try_reserve(control.table.living().count())
        .map_err(|_| "live task capacity")?;
    for row in control.table.living() {
        if let Slot::Live { task, .. } = row.slot
            && matches!(
                row.state,
                State::NeverStarted | State::Starting | State::Ready | State::Debarked
            )
            && !runtime::env::unit::join(task, Wait::POLL).unwrap_or(true)
        {
            living.push(task);
        }
    }
    let me = runtime::env::unit::self_id();
    let authority = current_authority(roster);
    let host = tree.host();
    let live = |task: TaskId| {
        task == me || Some(task) == authority || Some(task) == host || living.contains(&task)
    };
    publications.sweep(tree, live)?;
    names.sweep(roster, tree, live)?;
    resources.remove(tree, live)?;
    resources.prepare(&control.table, roster, tree)?;
    names.prepare(&control.table, roster, tree)?;
    publications.poll(
        entry,
        &control.table,
        roster,
        machine,
        resources,
        names,
        tree,
    )?;
    names.poll(roster);
    Ok(())
}
