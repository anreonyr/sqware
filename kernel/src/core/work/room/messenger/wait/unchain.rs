use alloc::sync::Arc;

use crate::core::work::unit::task::Task;

use super::holder::void;

pub(super) struct Unchain {
    cur: Option<Arc<Task>>,
}

impl Unchain {
    pub(super) fn new(cur: Option<Arc<Task>>) -> Self {
        Self { cur }
    }
}

impl Iterator for Unchain {
    type Item = Arc<Task>;

    fn next(&mut self) -> Option<Arc<Task>> {
        let task = self.cur.take()?;
        void(task.wait_ticket());
        self.cur = task.take_next();
        Some(task)
    }
}
