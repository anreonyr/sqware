use crate::system::control::identity::{Roster, current_authority};
use crate::system::control::unit::Control;
use crate::system::control::unit::table::{Slot, State};
use crate::system::operator::management::Tree;
use ::schedule::{Progress, Res, ResMut};
use alloc::vec::Vec;
use env::{TaskId, Wait};

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
        .try_reserve(control.living_count() + control.instances().count() + 3)
        .map_err(|_| "live task capacity")?;
    living.tasks.push(env::unit::self_id());

    for row in control.living() {
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
    living.tasks.extend(
        control
            .instances()
            .filter(|item| control.live(item.task))
            .map(|item| item.task),
    );
    Ok(Progress::Done)
}

pub(crate) fn authority(
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
