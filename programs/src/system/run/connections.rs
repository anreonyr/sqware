use alloc::vec::Vec;
use env::TaskId;
use ::schedule::{Progress, Res, ResMut};
use crate::system::operator::client::Tree;

pub struct Connections(pub Vec<TaskId>);
pub(crate) fn candidates(
    control: Res<crate::system::control::serve::unit::Control>,
    mut connections: ResMut<Connections>,
) -> Result<Progress, &'static str> {
    for task in control.tasks() {
        if !connections.0.contains(&task) {
            connections
                .0
                .try_reserve(1)
                .map_err(|_| "operator request capacity")?;
            connections.0.push(task);
        }
    }
    Ok(Progress::Done)
}
pub(crate) fn connect(
    mut connections: ResMut<Connections>,
    mut tree: ResMut<Tree>,
) -> Result<Progress, &'static str> {
    tree.connect(&mut connections.0)?;
    Ok(Progress::Done)
}
