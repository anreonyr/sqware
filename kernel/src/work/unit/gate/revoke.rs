use alloc::sync::Weak;

use env::{PieFail, PieToken};

use super::cull;
use crate::work::unit::task::Task;

pub(crate) fn revoke(
    caller: &Task,
    target: &Weak<Task>,
    token: PieToken,
) -> Result<usize, PieFail> {
    let _graph = super::GRAPH.lock();
    let snap = super::snap();
    let target = target.upgrade().ok_or(PieFail::Denied)?;
    let sire = {
        let pies = target.pies.lock();
        let pie = pies
            .iter()
            .find(|p| p.token() == token)
            .ok_or(PieFail::Denied)?;
        pie.sire().ok_or(PieFail::Denied)?
    };
    let mine = {
        let pies = caller.pies.lock();
        pies.iter().any(|p| p.token() == sire)
    };
    if !mine {
        return Err(PieFail::Denied);
    }
    Ok(cull::cull((target, token), &snap))
}
