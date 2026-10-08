use super::unit::Control;
use crate::system::control::core::unit::{Service, Slot, State};
use env::{TaskId, Wait};

impl Control {
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
        crate::system::control::core::verdict::walking(&self.table)
    }

    pub(crate) fn due(&self) -> bool {
        crate::system::control::core::verdict::due(&self.table)
    }

    pub(crate) fn done(&self) -> bool {
        crate::system::control::core::verdict::done(&self.table)
    }

    pub(crate) fn closing_service_names(&self) -> impl Iterator<Item = &str> {
        self.living().filter_map(|row| {
            (matches!(row.state, State::Starting | State::Ready | State::Debarked)
                && matches!(row.slot, Slot::Live { team: Some(_), .. }))
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
                    && !env::unit::join(task, Wait::POLL).unwrap_or(true)
            })
    }
}
