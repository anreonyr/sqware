use crate::system::operator::management::Tree;
use ::schedule::{Progress, Res, ResMut};
use alloc::vec::Vec;
use env::TaskId;

pub(super) struct Connections(Vec<TaskId>);
impl Connections {
    pub(super) fn new() -> Self {
        Self(Vec::new())
    }
    pub(super) fn request(&mut self, task: TaskId) -> Result<(), &'static str> {
        self.0
            .try_reserve(1)
            .map_err(|_| "operator request capacity")?;
        self.0.push(task);
        Ok(())
    }
}
pub(crate) fn candidates(
    control: Res<crate::system::control::unit::Control>,
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
