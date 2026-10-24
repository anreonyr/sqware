use alloc::vec::Vec;

use crate::system::control::unit::{
    table::{Slot, Table},
    verdict::Fail,
};
use crate::unit::Target;

use super::Input;

pub struct Control {
    pub table: Table,
    pub inputs: Vec<Input>,
    pub targets: &'static [Target],
    pub guarded: &'static [&'static str],
}

impl Control {
    pub fn input(&self, name: &str) -> Result<Input, Fail> {
        self.inputs
            .iter()
            .copied()
            .find(|input| input.program.name == name)
            .ok_or(Fail::Unknown)
    }

    pub fn task(&self, name: &str) -> Option<crate::TaskId> {
        match self.table.find(name)?.slot {
            Slot::Live { task, .. } => Some(task),
            Slot::None => None,
        }
    }

    pub fn live(&self, task: crate::TaskId) -> bool {
        (task == crate::unit::self_id() || self.task_of(task))
            && !crate::unit::join_task(task, crate::Wait::POLL).unwrap_or(true)
    }

    fn task_of(&self, task: crate::TaskId) -> bool {
        self.table.rows().any(|row| {
            matches!(row.slot, Slot::Live { task: live, .. } if live == task)
        })
    }

    pub fn guard_required(&self, name: &str) -> bool {
        self.guarded.contains(&name)
    }
}
