pub struct Execution {
    pub instance: Option<crate::instance::Instance>,
    pub task: Option<crate::TaskId>,
    pub deadline: u64,
}
