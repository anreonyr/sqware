use super::Control;
use crate::system::control::unit::table::{Service, Slot, State};
use env::{TaskId, Wait};

impl Control {
    pub(crate) fn named_task(&self, name: &str) -> Result<TaskId, super::verdict::Fail> {
        let task = self.task(name).ok_or(super::verdict::Fail::Unknown)?;
        if task == TaskId::new(0) || !self.live(task) {
            return Err(super::verdict::Fail::NotReady);
        }
        Ok(task)
    }

    pub(crate) fn living(&self) -> impl Iterator<Item = &Service> {
        self.table.living()
    }

    pub(crate) fn living_count(&self) -> usize {
        self.living().count()
    }

    pub(crate) fn find_named_task(&self, task: TaskId) -> Option<&Service> {
        self.living()
            .find(|row| matches!(row.slot, Slot::Live { task: known, .. } if known == task))
    }

    pub(crate) fn walking(&self) -> bool {
        crate::system::control::unit::verdict::walking(&self.table)
    }

    pub(crate) fn due(&self) -> bool {
        crate::system::control::unit::verdict::due(&self.table)
    }

    pub(crate) fn done(&self) -> bool {
        crate::system::control::unit::verdict::done(&self.table)
    }

    pub(crate) fn closing_service_names(&self) -> impl Iterator<Item = &str> {
        self.living().filter_map(|row| {
            (matches!(row.state, State::Starting | State::Ready | State::Debarked)
                && matches!(row.slot, Slot::Live { team: Some(_), .. })
                && !self.living().any(|other| {
                    if other.name == row.name || !matches!(other.slot, Slot::Live { .. }) {
                        return false;
                    }
                    let Ok(input) = self.input(&other.name) else { return false };
                    // Reuse the declaration: stop users before their providers.
                    // Stopping users still hold the provider until reclaim finishes.
                    input.program.relation.after.is_some_and(|after| {
                        after.iter().any(|name| *name == row.name)
                            || (after.iter().any(|name| crate::unit::is_target(name))
                                && self.input(&row.name).is_ok_and(|input| {
                                    !input.program.relation.after.is_some_and(|after| {
                                        after.iter().any(|name| crate::unit::is_target(name))
                                    })
                                }))
                    })
                }))
            .then_some(row.name.as_str())
        })
    }

    pub(crate) fn live_service(&self, task: TaskId) -> bool {
        task == env::unit::self_id()
            || self.living().any(|row| {
                matches!(row.slot, Slot::Live { task: known, .. } if known == task)
                    && matches!(
                        row.state,
                        State::NeverStarted | State::Starting | State::Ready | State::Debarked
                    )
                    && !env::unit::join_task(task, Wait::POLL).unwrap_or(true)
            })
    }
}
