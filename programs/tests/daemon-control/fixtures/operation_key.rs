use super::{Request, Waiters};

pub struct OperationKey {
    pub request: Request,
    pub waiters: Waiters,
}
