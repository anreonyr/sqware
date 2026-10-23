use super::Execution;
use crate::system::control::unit::verdict::Fail;

pub struct ExecutionProgress {
    pub execution: Execution,
    pub failure: Option<Fail>,
}
