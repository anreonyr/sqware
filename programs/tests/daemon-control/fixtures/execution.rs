use super::Instance;

pub struct Execution {
    pub instance: Option<Instance>,
    pub task: Option<crate::TaskId>,
    pub deadline: u64,
}
