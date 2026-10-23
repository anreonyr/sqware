use super::{ExecutionProgress, OperationKey};

pub struct Operation {
    pub key: OperationKey,
    pub progress: ExecutionProgress,
}
