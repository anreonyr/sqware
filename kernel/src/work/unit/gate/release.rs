use env::{PieFail, PieToken};

use super::cull;
use super::snap::Snap;
use crate::work::unit::task::Task;

pub(crate) fn release(
    task: &alloc::sync::Arc<Task>,
    token: PieToken,
    snap: &Snap,
) -> Result<usize, PieFail> {
    let present = {
        let pies = task.pies.lock();
        pies.iter().any(|p| p.token() == token)
    };
    if !present {
        return Err(PieFail::Denied);
    }
    Ok(cull::cull((task.clone(), token), snap))
}