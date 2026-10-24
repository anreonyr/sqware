use env::TaskId;

use super::Instance;

pub struct ControlFace;

impl ControlFace {
    pub fn instance(&self, task: TaskId) -> Instance {
        Instance { task }
    }
}
