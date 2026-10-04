use super::unit::Control;
use crate::system::control::core::unit::{Slot, State};
use crate::system::identity::serve::{install::Roster, query::current_authority};
use crate::system::operator::serve::install::Tree;
use alloc::vec::Vec;
use env::{TaskId, Wait};
use protocol::common::schedule::{Progress, Res, ResMut};

pub struct Living {
    tasks: Vec<TaskId>,
}
impl Living {
    pub fn new() -> Self {
        Self { tasks: Vec::new() }
    }
    pub fn contains(&self, task: TaskId) -> bool {
        self.tasks.contains(&task)
    }
}
pub fn capture(
    control: Res<Control>,
    mut living: ResMut<Living>,
) -> Result<Progress, &'static str> {
    living.tasks.clear();
    living
        .tasks
        .try_reserve(control.table.living().count() + 3)
        .map_err(|_| "live task capacity")?;
    living.tasks.push(env::unit::self_id());

    for row in control.table.living() {
        if let Slot::Live { task, .. } = row.slot
            && matches!(
                row.state,
                State::NeverStarted | State::Starting | State::Ready | State::Debarked
            )
            && !env::unit::join(task, Wait::POLL).unwrap_or(true)
        {
            living.tasks.push(task);
        }
    }
    Ok(Progress::Done)
}

pub fn authority(
    roster: Res<Roster>,
    mut living: ResMut<Living>,
) -> Result<Progress, &'static str> {
    living.tasks.extend(current_authority(&roster));
    Ok(Progress::Done)
}
pub fn operator(tree: Res<Tree>, mut living: ResMut<Living>) -> Result<Progress, &'static str> {
    living.tasks.extend(tree.host());
    Ok(Progress::Done)
}
