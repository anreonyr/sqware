use crate::work::unit::task::Task;
use alloc::sync::{Arc, Weak};
use env::{PieFail, PieToken};

pub(crate) fn revoke(
    caller: &Arc<Task>,
    target: &Weak<Task>,
    token: PieToken,
) -> Result<usize, PieFail> {
    let target = target.upgrade().ok_or(PieFail::Denied)?;
    super::cull::cull(&target, token, Some(caller), false).map(|cleanup| cleanup.finish())
}
