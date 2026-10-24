use env::TaskId;

mod instance;
pub use instance::Instance;

pub struct Face {
    pub task: TaskId,
}

impl Face {
    pub fn instance(&self, task: TaskId) -> Instance {
        Instance { task }
    }
}
