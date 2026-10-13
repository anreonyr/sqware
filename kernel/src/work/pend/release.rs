use crate::work::unit::task::Task;
use alloc::sync::Arc;
use env::{PieFail, PieToken};

pub(crate) fn release(task: &Arc<Task>, token: PieToken) -> Result<usize, PieFail> {
    super::cull::cull(task, token, None, false).map(|cleanup| cleanup.finish())
}
