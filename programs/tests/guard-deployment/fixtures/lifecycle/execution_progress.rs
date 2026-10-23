pub struct ExecutionProgress {
    pub execution: crate::execution::Execution,
    pub failure: Option<crate::system::control::unit::verdict::Fail>,
}
