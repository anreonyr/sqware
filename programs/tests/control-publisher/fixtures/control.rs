use alloc::vec::Vec;
use env::TaskId;

use crate::fixtures::input::Input;
use crate::unit::table::Table;

pub struct Control {
    pub inputs: Vec<Input>,
    pub table: Table,
    pub dead: Vec<TaskId>,
}

impl Control {
    pub fn new(inputs: Vec<Input>) -> Self {
        Self {
            inputs,
            table: Table::new(),
            dead: Vec::new(),
        }
    }

    pub fn live(&self, task: TaskId) -> bool {
        !self.dead.contains(&task)
    }
}
