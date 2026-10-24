use super::authority::{group, supervisor};
use crate::system::app::policy::Settle;
use crate::system::control::unit::{
    Control,
    table::{Slot, State},
    verdict::Fail,
};
use env::TaskId;

pub(super) struct Verify<'a> {
    pub control: &'a Control,
    pub settle: &'a Settle,
    pub caller: TaskId,
    pub target: &'a str,
}

impl Verify<'_> {
    pub(super) fn run(&self, retired: bool) -> Result<(), Fail> {
        if self.settle.closing() {
            return Err(Fail::Denied);
        }
        let source = supervisor(self.control, self.caller)?;
        let group = group(self.control, source.name.as_str())?;
        if !group.members.contains(&self.target) || source.name == self.target {
            return Err(Fail::Denied);
        }
        if self.control.guard_required(self.target) {
            return Err(Fail::Denied);
        }
        let row = self.control.table.find(self.target).ok_or(Fail::Denied)?;
        if !row.run_requested {
            return Err(Fail::Denied);
        }
        if retired && (row.state != State::Dead || !matches!(row.slot, Slot::None)) {
            return Err(Fail::Denied);
        }
        Ok(())
    }
}
